mod config;
mod list;
mod persistence;
mod registry;
mod status;

pub use config::{
    build_scratchpad_keybind_reconfigure, build_scratchpad_keybind_update,
    is_valid_scratchpad_name, parse_scratchpad_action, parse_scratchpads_kdl, AxisOrigin, Origin,
    ResolvedCoordinates, ScratchpadAction, ScratchpadActionTarget, ScratchpadConfig,
    ScratchpadKeybind, ScratchpadKeybindAction, ScratchpadKeybindRebind, ScratchpadKeybindUnbind,
};
pub use list::{ScratchpadInstanceInfo, ScratchpadListEntry, ScratchpadListQuery};
pub use persistence::{delete_state_file, load_state, save_state, PersistedState};
pub use registry::{
    acquire_registry_lock, registry_file_path, registry_lock_path, registry_temp_file_path,
    OpenDecision, RegistryFileLock, RegistryLockMetadata, RegistryRecord, RegistryRecordState,
    ScratchpadRegistry,
};
pub use status::{ScratchpadDisplayState, ScratchpadStatusItem, ScratchpadStatusSnapshot};

use std::collections::{HashMap, HashSet};
use zellij_tile::prelude::{CommandToRun, PaneInfo};

/// Commands returned by ScratchpadManager for the caller to execute
#[derive(Debug)]
pub enum ScratchpadCommand {
    /// Open a new floating pane with the given command and resolved coordinates.
    /// Carries the scratchpad `name` and `tab_id` so the caller can register
    /// the pane once `open_command_pane_floating` returns the `PaneId`.
    OpenFloating {
        command: CommandToRun,
        coordinates: ResolvedCoordinates,
        name: String,
        tab_id: usize,
    },
    /// Show a pane (make visible and focus), optionally re-applying coordinates
    ShowPane {
        pane_id: u32,
        tab_id: usize,
        coordinates: Option<ResolvedCoordinates>,
    },
    /// Hide a pane (suppress)
    HidePane { pane_id: u32 },
    /// Close a pane
    ClosePane { pane_id: u32 },

    /// Rename a pane (set its title in the Zellij UI)
    RenamePane { pane_id: u32, name: String },
}

/// Context needed by ScratchpadManager to make decisions
pub struct ScratchpadContext<'a> {
    pub pane_manifest: &'a HashMap<usize, Vec<PaneInfo>>,
    pub current_tab_position: usize,
    pub current_tab_id: Option<usize>,
    pub are_floating_panes_visible: bool,
    pub tab_id_to_position: &'a HashMap<usize, usize>,
    /// Viewport width in columns (from TabInfo).
    pub viewport_cols: usize,
    /// Viewport height in rows (from TabInfo).
    pub viewport_rows: usize,
}

/// Manages scratchpad state and returns commands to execute
pub struct ScratchpadManager {
    configs: HashMap<String, ScratchpadConfig>,
    /// name -> (tab_id -> pane_id)
    panes: HashMap<String, HashMap<usize, u32>>,
    /// Display-only observations, never consumed by actions or reconciliation.
    passive_panes: HashMap<String, HashMap<usize, u32>>,
    /// tab_id -> (last focused passive name, pane_id, local focus counter at observation).
    passive_mru: HashMap<usize, (String, u32, u64)>,
    /// Monotonic counter for focus tracking
    focus_counter: u64,
    /// name -> (tab_id -> last focus timestamp)
    focus_times: HashMap<String, HashMap<usize, u64>>,
    /// Track scratchpad we just showed (for focus detection before PaneUpdate)
    just_shown: Option<u32>,
    /// Scratchpads that were removed from config but still have active panes
    /// name -> set of tab_ids
    orphaned: HashMap<String, HashSet<usize>>,
}

impl ScratchpadManager {
    pub fn new(configs: HashMap<String, ScratchpadConfig>) -> Self {
        Self {
            configs,
            panes: HashMap::new(),
            passive_panes: HashMap::new(),
            passive_mru: HashMap::new(),
            focus_counter: 0,
            focus_times: HashMap::new(),
            just_shown: None,
            orphaned: HashMap::new(),
        }
    }

    /// Clear the "just shown" tracking (call on PaneUpdate)
    pub fn clear_just_shown(&mut self) {
        self.just_shown = None;
    }

    /// Reconcile state with new configuration after a config reload.
    /// Returns commands to show orphaned panes.
    pub fn reconcile_config(
        &mut self,
        new_configs: HashMap<String, ScratchpadConfig>,
    ) -> Vec<ScratchpadCommand> {
        let mut commands = Vec::new();

        // Find scratchpads that were removed from config but have active panes
        let removed_names: HashSet<&String> = self
            .configs
            .keys()
            .filter(|name| !new_configs.contains_key(*name))
            .collect();

        for (name, inner) in &self.panes {
            if removed_names.contains(name) {
                for (&tab_id, &pane_id) in inner {
                    let orphaned_set = self.orphaned.get(name);
                    if !orphaned_set.is_some_and(|s| s.contains(&tab_id)) {
                        commands.push(ScratchpadCommand::ShowPane {
                            pane_id,
                            tab_id,
                            coordinates: None,
                        });
                        // Defer insertion to avoid borrow conflict
                    }
                }
            }
        }

        // Now insert orphaned entries (separate loop to avoid borrow conflict)
        for name in &removed_names {
            if let Some(inner) = self.panes.get(*name) {
                let tab_ids: Vec<usize> = inner.keys().copied().collect();
                for tab_id in tab_ids {
                    let orphaned_set = self.orphaned.entry((*name).clone()).or_default();
                    if orphaned_set.insert(tab_id) {
                        eprintln!(
                            "Scratchpad '{}' removed from config but has active panes",
                            name
                        );
                    }
                }
            }
        }

        // Un-orphan scratchpads that were re-added to config
        let readded: Vec<String> = self
            .orphaned
            .keys()
            .filter(|name| new_configs.contains_key(*name))
            .cloned()
            .collect();

        for name in readded {
            self.orphaned.remove(&name);
            eprintln!(
                "Scratchpad '{}' re-added to config, resuming management",
                name
            );
        }

        self.configs = new_configs;
        commands
    }

    /// Check if a scratchpad is orphaned (removed from config but has active panes)
    fn is_orphaned(&self, name: &str, tab_id: usize) -> bool {
        self.orphaned.get(name).is_some_and(|s| s.contains(&tab_id))
    }

    /// Extract state that should be persisted across reloads.
    pub fn persisted_state(&self) -> PersistedState {
        PersistedState {
            panes: self
                .panes
                .iter()
                .flat_map(|(name, inner)| {
                    inner
                        .iter()
                        .map(move |(&tab_id, &pane_id)| ((name.clone(), tab_id), pane_id))
                })
                .collect(),
            focus_times: self
                .focus_times
                .iter()
                .flat_map(|(name, inner)| {
                    inner
                        .iter()
                        .map(move |(&tab_id, &time)| ((name.clone(), tab_id), time))
                })
                .collect(),
            focus_counter: self.focus_counter,
        }
    }

    /// Restore state from a previous session.
    /// Returns commands to show orphaned panes (panes whose scratchpad names are not in current config).
    pub fn restore_state(&mut self, state: PersistedState) -> Vec<ScratchpadCommand> {
        self.panes.clear();
        for ((name, tab_id), pane_id) in state.panes {
            self.panes.entry(name).or_default().insert(tab_id, pane_id);
        }
        self.focus_times.clear();
        for ((name, tab_id), time) in state.focus_times {
            self.focus_times
                .entry(name)
                .or_default()
                .insert(tab_id, time);
        }
        self.focus_counter = state.focus_counter;

        // Detect orphaned scratchpads - panes whose names aren't in current config
        let mut commands = Vec::new();
        for (name, inner) in &self.panes {
            if !self.configs.contains_key(name) {
                for (&tab_id, &pane_id) in inner {
                    let is_already_orphaned =
                        self.orphaned.get(name).is_some_and(|s| s.contains(&tab_id));
                    if !is_already_orphaned {
                        commands.push(ScratchpadCommand::ShowPane {
                            pane_id,
                            tab_id,
                            coordinates: None,
                        });
                        self.orphaned
                            .entry(name.clone())
                            .or_default()
                            .insert(tab_id);
                        eprintln!(
                            "Scratchpad '{}' removed from config but has active panes",
                            name
                        );
                    }
                }
            }
        }
        commands
    }

    /// Handle a scratchpad action, returns commands to execute
    pub fn handle_action(
        &mut self,
        action: ScratchpadAction,
        ctx: &ScratchpadContext,
    ) -> Vec<ScratchpadCommand> {
        match action {
            ScratchpadAction::Toggle { name, .. } => self.handle_toggle(name, ctx),
            ScratchpadAction::Show { name, .. } => self.handle_show(&name, ctx),
            ScratchpadAction::Hide { name, .. } => self.handle_hide(&name, ctx),
            ScratchpadAction::Close { name, .. } => self.handle_close(&name, ctx),
        }
    }

    /// If the given terminal pane belongs to a scratchpad, run the show logic
    /// to ensure its size/position is correct and return the commands.
    /// Returns `None` if the pane is not a known scratchpad.
    pub fn handle_focus_pane(
        &mut self,
        terminal_pane_id: u32,
        ctx: &ScratchpadContext,
    ) -> Option<Vec<ScratchpadCommand>> {
        // Reverse-look up the scratchpad name and tab from the pane ID
        let (name, tab_id) = self.find_scratchpad_by_pane_id(terminal_pane_id)?;

        // Skip orphaned scratchpads
        if self.is_orphaned(&name, tab_id) {
            return None;
        }

        let config = self.configs.get(&name)?.clone();
        let coordinates = config.resolve_coordinates(ctx.viewport_cols, ctx.viewport_rows);
        Some(self.show_pane(&name, terminal_pane_id, tab_id, ctx, coordinates))
    }

    /// Called on PaneUpdate to sync state and clean up
    pub fn on_pane_update(
        &mut self,
        ctx: &ScratchpadContext,
        orphaned_tabs: &HashSet<usize>,
    ) -> Vec<ScratchpadCommand> {
        let mut commands = Vec::new();

        // Track focus changes that happen outside of our actions
        self.update_focus_tracking(ctx);

        // Close exited scratchpads and clean up closed panes in a single pass
        commands.extend(self.cleanup_panes(ctx));

        // Close scratchpads for orphaned tabs
        commands.extend(self.close_orphaned_scratchpads(orphaned_tabs));

        commands
    }

    fn handle_toggle(
        &mut self,
        name: Option<String>,
        ctx: &ScratchpadContext,
    ) -> Vec<ScratchpadCommand> {
        let target_name = match name {
            Some(n) => n,
            None => {
                if let Some(focused) = self.get_focused_scratchpad(ctx) {
                    return self.handle_hide(&focused, ctx);
                }
                return match self.get_last_focused_on_current_tab(ctx) {
                    Some(last) => self.handle_show(&last, ctx),
                    None => Vec::new(),
                };
            }
        };

        // Check if configured
        if !self.configs.contains_key(&target_name) {
            return Vec::new();
        }

        // No-op for orphaned scratchpads
        if let Some(tab_id) = ctx.current_tab_id {
            if self.is_orphaned(&target_name, tab_id) {
                return Vec::new();
            }
        }

        let visible = self.is_visible(&target_name, ctx);
        let focused = self.is_focused(&target_name, ctx);

        if visible && focused {
            self.handle_hide(&target_name, ctx)
        } else {
            self.handle_show(&target_name, ctx)
        }
    }

    fn handle_show(&mut self, name: &str, ctx: &ScratchpadContext) -> Vec<ScratchpadCommand> {
        let config = match self.configs.get(name) {
            Some(c) => c.clone(),
            None => return Vec::new(),
        };

        let Some(tab_id) = ctx.current_tab_id else {
            return Vec::new();
        };

        // No-op for orphaned scratchpads
        if self.is_orphaned(name, tab_id) {
            return Vec::new();
        }

        let existing_pane_id = self.get_pane(name, ctx);

        if existing_pane_id.is_none() {
            let program = &config.command[0];
            let args: Vec<String> = config.command[1..].to_vec();
            let mut cmd = CommandToRun::new_with_args(program, args);
            if let Some(ref cwd) = config.cwd {
                cmd.cwd = Some(std::path::PathBuf::from(cwd));
            }
            let coordinates = config.resolve_coordinates(ctx.viewport_cols, ctx.viewport_rows);
            return vec![ScratchpadCommand::OpenFloating {
                command: cmd,
                coordinates,
                name: name.to_string(),
                tab_id,
            }];
        }

        let Some(pane_id) = existing_pane_id else {
            unreachable!("guarded by is_none check above");
        };
        let coordinates = config.resolve_coordinates(ctx.viewport_cols, ctx.viewport_rows);
        self.show_pane(name, pane_id, tab_id, ctx, coordinates)
    }

    fn handle_hide(&mut self, name: &str, ctx: &ScratchpadContext) -> Vec<ScratchpadCommand> {
        // No-op for orphaned scratchpads
        if let Some(tab_id) = ctx.current_tab_id {
            if self.is_orphaned(name, tab_id) {
                return Vec::new();
            }
        }

        if let Some(pane_id) = self.get_pane(name, ctx) {
            vec![ScratchpadCommand::HidePane { pane_id }]
        } else {
            Vec::new()
        }
    }

    fn handle_close(&mut self, name: &str, ctx: &ScratchpadContext) -> Vec<ScratchpadCommand> {
        let Some(tab_id) = ctx.current_tab_id else {
            return Vec::new();
        };

        let pane_id = self
            .panes
            .get_mut(name)
            .and_then(|inner| inner.remove(&tab_id));

        if let Some(pane_id) = pane_id {
            // Clean up empty outer entries
            if self.panes.get(name).is_some_and(|m| m.is_empty()) {
                self.panes.remove(name);
            }
            if let Some(inner) = self.focus_times.get_mut(name) {
                inner.remove(&tab_id);
                if inner.is_empty() {
                    self.focus_times.remove(name);
                }
            }
            vec![ScratchpadCommand::ClosePane { pane_id }]
        } else {
            Vec::new()
        }
    }

    /// Merge externally-known scratchpad panes (from the shared registry) into
    /// the local pane map.
    ///
    /// In a multi-client session each client has its own plugin instance with
    /// its own `panes` map. A scratchpad opened by one client's instance is only
    /// recorded in that instance's map plus the shared registry. Before another
    /// instance makes a toggle/show/hide decision it must learn about those
    /// panes, otherwise it wrongly believes no pane exists and tries to open one.
    pub fn sync_known_panes(&mut self, known: &[(String, usize, u32)]) {
        for (name, tab_id, pane_id) in known {
            self.panes
                .entry(name.clone())
                .or_default()
                .insert(*tab_id, *pane_id);
        }
    }

    /// Prune display-only observations using complete pane and authoritative tab snapshots.
    /// Call before attempting registry I/O, even when that read fails. Wait until both
    /// snapshots are available. Pane liveness ignores positions to tolerate reorder lag.
    /// Local action ownership and focus history are never changed.
    pub fn prune_passive_observations(&mut self, ctx: &ScratchpadContext<'_>) {
        let live_panes: HashSet<u32> = ctx
            .pane_manifest
            .values()
            .flatten()
            .filter(|pane| !pane.is_plugin && !pane.exited && !pane.is_held)
            .map(|pane| pane.id)
            .collect();
        self.passive_panes.retain(|name, panes| {
            if !self.configs.contains_key(name) {
                return false;
            }
            panes.retain(|tab_id, pane_id| {
                ctx.tab_id_to_position.contains_key(tab_id) && live_panes.contains(pane_id)
            });
            !panes.is_empty()
        });
        self.passive_mru.retain(|tab_id, (name, pane_id, _)| {
            self.passive_panes
                .get(name)
                .and_then(|panes| panes.get(tab_id))
                == Some(pane_id)
        });
    }

    /// Discover configured, observed Present records without reconciling or writing
    /// the shared registry. Only display-only observations and MRU are updated;
    /// action ownership, focus history, and orphan tracking are untouched. Empty
    /// registry reads retain observations. Panes never transfer between name/tab keys.
    pub fn discover_panes_from_registry(
        &mut self,
        registry: &ScratchpadRegistry,
        ctx: &ScratchpadContext<'_>,
    ) {
        let observed: HashSet<(usize, u32)> = ctx
            .pane_manifest
            .iter()
            .flat_map(|(&position, panes)| {
                panes
                    .iter()
                    .filter(|pane| !pane.is_plugin && !pane.exited && !pane.is_held)
                    .map(move |pane| (position, pane.id))
            })
            .collect();
        let mut known_panes: HashSet<u32> = self
            .panes
            .values()
            .chain(self.passive_panes.values())
            .flat_map(|panes| panes.values().copied())
            .collect();

        for record in &registry.entries {
            let RegistryRecordState::Present { pane_id } = record.state else {
                continue;
            };
            if !self.configs.contains_key(&record.name)
                || self
                    .panes
                    .get(&record.name)
                    .is_some_and(|panes| panes.contains_key(&record.tab_id))
                || !ctx
                    .tab_id_to_position
                    .get(&record.tab_id)
                    .is_some_and(|&position| observed.contains(&(position, pane_id)))
                || !known_panes.insert(pane_id)
            {
                continue;
            }
            self.passive_panes
                .entry(record.name.clone())
                .or_default()
                .insert(record.tab_id, pane_id);
        }

        let view = self.publication_view();
        for (&tab_id, &position) in ctx.tab_id_to_position {
            let tab_ctx = ScratchpadContext {
                current_tab_id: Some(tab_id),
                current_tab_position: position,
                ..*ctx
            };
            let Some(name) = view.get_focused_scratchpad(&tab_ctx) else {
                continue;
            };
            if self
                .panes
                .get(&name)
                .is_some_and(|panes| panes.contains_key(&tab_id))
            {
                if self
                    .passive_mru
                    .get(&tab_id)
                    .is_some_and(|(mru_name, pane_id, counter)| {
                        *mru_name != name
                            || view.get_pane(&name, &tab_ctx) != Some(*pane_id)
                            || self
                                .focus_times
                                .values()
                                .any(|times| times.get(&tab_id).is_some_and(|time| time > counter))
                    })
                {
                    self.passive_mru.remove(&tab_id);
                }
            } else if let Some(pane_id) = view.get_pane(&name, &tab_ctx) {
                self.passive_mru
                    .insert(tab_id, (name, pane_id, self.focus_counter));
            }
        }
    }

    /// Register a pane that was just opened for a scratchpad.
    /// Called by the plugin after `open_command_pane_floating` returns the `PaneId`.
    pub fn register_pane(
        &mut self,
        name: &str,
        tab_id: usize,
        pane_id: u32,
    ) -> Vec<ScratchpadCommand> {
        // Record the mapping
        self.panes
            .entry(name.to_string())
            .or_default()
            .insert(tab_id, pane_id);

        self.just_shown = Some(pane_id);
        self.focus_counter += 1;
        self.focus_times
            .entry(name.to_string())
            .or_default()
            .insert(tab_id, self.focus_counter);

        // Rename pane: use configured title, or fall back to scratchpad name
        let title = self
            .configs
            .get(name)
            .and_then(|c| c.title.clone())
            .unwrap_or_else(|| name.to_string());

        vec![ScratchpadCommand::RenamePane {
            pane_id,
            name: title,
        }]
    }

    fn show_pane(
        &mut self,
        name: &str,
        pane_id: u32,
        tab_id: usize,
        ctx: &ScratchpadContext,
        coordinates: ResolvedCoordinates,
    ) -> Vec<ScratchpadCommand> {
        let mut commands = Vec::new();
        let hidden_before = self.get_hidden_floating_pane_ids(ctx);

        commands.push(ScratchpadCommand::ShowPane {
            pane_id,
            tab_id,
            coordinates: Some(coordinates),
        });
        self.just_shown = Some(pane_id);

        // Re-hide panes that were hidden before
        for hidden_pane_id in hidden_before {
            if hidden_pane_id != pane_id {
                commands.push(ScratchpadCommand::HidePane {
                    pane_id: hidden_pane_id,
                });
            }
        }

        self.focus_counter += 1;
        self.focus_times
            .entry(name.to_string())
            .or_default()
            .insert(tab_id, self.focus_counter);

        commands
    }

    fn get_pane(&self, name: &str, ctx: &ScratchpadContext) -> Option<u32> {
        let tab_id = ctx.current_tab_id?;
        self.panes.get(name)?.get(&tab_id).copied()
    }

    /// Reverse-look up a terminal pane ID to find the scratchpad name and tab it belongs to.
    fn find_scratchpad_by_pane_id(&self, terminal_pane_id: u32) -> Option<(String, usize)> {
        self.panes
            .iter()
            .flat_map(|(name, inner)| {
                inner
                    .iter()
                    .map(move |(&tab_id, &pid)| (name.clone(), tab_id, pid))
            })
            .find(|(_, _, pid)| *pid == terminal_pane_id)
            .map(|(name, tab_id, _)| (name, tab_id))
    }

    fn get_hidden_floating_pane_ids(&self, ctx: &ScratchpadContext) -> HashSet<u32> {
        ctx.pane_manifest
            .values()
            .flatten()
            .filter(|p| p.is_floating && p.is_suppressed)
            .map(|p| p.id)
            .collect()
    }

    fn is_visible(&self, name: &str, ctx: &ScratchpadContext) -> bool {
        let Some(pane_id) = self.get_pane(name, ctx) else {
            return false;
        };

        // Decide purely from the pane manifest, which is a single coherent
        // PaneUpdate snapshot. A scratchpad hidden via `hide_pane_with_id` is
        // marked `is_suppressed` in that same snapshot, so we don't need the
        // separately-delivered tab-level `are_floating_panes_visible` flag.
        // That flag arrives via a different event (TabUpdate) and can be stale
        // relative to the manifest, which produced wrong toggle decisions.
        ctx.pane_manifest.values().flatten().any(|p| {
            p.id == pane_id && p.is_floating && !p.is_suppressed && !p.exited && !p.is_held
        })
    }

    fn is_focused(&self, name: &str, ctx: &ScratchpadContext) -> bool {
        let Some(pane_id) = self.get_pane(name, ctx) else {
            return false;
        };

        if self.just_shown == Some(pane_id) {
            return true;
        }

        // `is_focused` in the manifest is only ever true while the floating
        // layer is visible (hiding it unfocuses all floating panes), so the
        // manifest alone is a coherent signal. We deliberately avoid the stale
        // tab-level `are_floating_panes_visible` flag here.
        ctx.pane_manifest
            .values()
            .flatten()
            .find(|p| p.id == pane_id && p.is_floating)
            .map(|p| p.is_focused)
            .unwrap_or(false)
    }

    fn get_focused_scratchpad(&self, ctx: &ScratchpadContext) -> Option<String> {
        let tab_id = ctx.current_tab_id?;
        if let Some(pane_id) = self.just_shown {
            if let Some((name, _)) = self
                .panes
                .iter()
                .find(|(_, panes)| panes.get(&tab_id) == Some(&pane_id))
            {
                return Some(name.clone());
            }
        }

        // A focused floating pane in the manifest already implies the floating
        // layer is visible. Other tabs can retain focused panes in the manifest.
        let focused_pane_id = ctx
            .pane_manifest
            .get(&ctx.current_tab_position)?
            .iter()
            .find(|p| {
                !p.is_plugin
                    && p.is_floating
                    && p.is_focused
                    && !p.is_suppressed
                    && !p.exited
                    && !p.is_held
            })?
            .id;

        self.panes
            .iter()
            .find(|(_, panes)| panes.get(&tab_id) == Some(&focused_pane_id))
            .map(|(name, _)| name.clone())
    }

    fn get_last_focused_on_current_tab(&self, ctx: &ScratchpadContext) -> Option<String> {
        let tab_id = ctx.current_tab_id?;

        self.focus_times
            .iter()
            .filter_map(|(name, inner)| inner.get(&tab_id).map(|&time| (name, time)))
            .max_by_key(|(_, focus_time)| *focus_time)
            .map(|(name, _)| name.clone())
    }

    fn update_focus_tracking(&mut self, ctx: &ScratchpadContext) {
        let Some(tab_id) = ctx.current_tab_id else {
            return;
        };
        if let Some(name) = self.get_focused_scratchpad(ctx) {
            self.focus_counter += 1;
            self.focus_times
                .entry(name)
                .or_default()
                .insert(tab_id, self.focus_counter);
        }
    }

    /// Single-pass cleanup: close exited/held panes and remove panes missing from manifest.
    /// Builds one HashSet of all pane IDs and one of exited/held IDs from a single scan.
    fn cleanup_panes(&mut self, ctx: &ScratchpadContext) -> Vec<ScratchpadCommand> {
        let mut all_pane_ids: HashSet<u32> = HashSet::new();
        let mut exited_pane_ids: HashSet<u32> = HashSet::new();

        for pane in ctx.pane_manifest.values().flatten() {
            all_pane_ids.insert(pane.id);
            if pane.exited || pane.is_held {
                exited_pane_ids.insert(pane.id);
            }
        }

        let mut commands = Vec::new();
        let keys_to_remove: Vec<(String, usize, u32)> = self
            .panes
            .iter()
            .flat_map(|(name, inner)| {
                inner
                    .iter()
                    .map(move |(&tab_id, &pane_id)| (name.clone(), tab_id, pane_id))
            })
            .filter(|(_, _, pane_id)| {
                // Remove if exited/held OR not in manifest at all
                exited_pane_ids.contains(pane_id) || !all_pane_ids.contains(pane_id)
            })
            .collect();

        for (name, tab_id, pane_id) in keys_to_remove {
            // Only emit ClosePane for exited/held panes (still in manifest)
            if exited_pane_ids.contains(&pane_id) {
                commands.push(ScratchpadCommand::ClosePane { pane_id });
            }
            if let Some(inner) = self.panes.get_mut(&name) {
                inner.remove(&tab_id);
                if inner.is_empty() {
                    self.panes.remove(&name);
                }
            }
            if let Some(inner) = self.focus_times.get_mut(&name) {
                inner.remove(&tab_id);
                if inner.is_empty() {
                    self.focus_times.remove(&name);
                }
            }
            if let Some(inner) = self.orphaned.get_mut(&name) {
                inner.remove(&tab_id);
                if inner.is_empty() {
                    self.orphaned.remove(&name);
                }
            }
        }

        commands
    }

    fn close_orphaned_scratchpads(
        &mut self,
        orphaned_tabs: &HashSet<usize>,
    ) -> Vec<ScratchpadCommand> {
        let orphaned_tab_panes: Vec<(String, usize, u32)> = self
            .panes
            .iter()
            .flat_map(|(name, inner)| {
                inner
                    .iter()
                    .map(move |(&tab_id, &pane_id)| (name.clone(), tab_id, pane_id))
            })
            .filter(|(_, tab_id, _)| orphaned_tabs.contains(tab_id))
            .collect();

        let mut commands = Vec::new();
        for (name, tab_id, pane_id) in orphaned_tab_panes {
            commands.push(ScratchpadCommand::ClosePane { pane_id });
            if let Some(inner) = self.panes.get_mut(&name) {
                inner.remove(&tab_id);
                if inner.is_empty() {
                    self.panes.remove(&name);
                }
            }
            if let Some(inner) = self.focus_times.get_mut(&name) {
                inner.remove(&tab_id);
                if inner.is_empty() {
                    self.focus_times.remove(&name);
                }
            }
            if let Some(inner) = self.orphaned.get_mut(&name) {
                inner.remove(&tab_id);
                if inner.is_empty() {
                    self.orphaned.remove(&name);
                }
            }
        }
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pane(id: u32, is_floating: bool, is_focused: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_floating,
            is_focused,
            ..Default::default()
        }
    }

    fn make_floating_pane(id: u32, is_focused: bool) -> PaneInfo {
        make_pane(id, true, is_focused)
    }

    fn make_config(cmd: &str) -> ScratchpadConfig {
        ScratchpadConfig {
            command: vec![cmd.to_string()],
            x: None,
            y: None,
            width: None,
            height: None,
            origin: Origin::default(),
            title: None,
            cwd: None,
            keybinds: Vec::new(),
        }
    }

    fn make_configs(names: &[&str]) -> HashMap<String, ScratchpadConfig> {
        names
            .iter()
            .map(|n| (n.to_string(), make_config("bash")))
            .collect()
    }

    fn make_context<'a>(
        pane_manifest: &'a HashMap<usize, Vec<PaneInfo>>,
        current_tab: usize,
        tab_id: Option<usize>,
        floating_visible: bool,
        tab_id_to_position: &'a HashMap<usize, usize>,
    ) -> ScratchpadContext<'a> {
        ScratchpadContext {
            pane_manifest,
            current_tab_position: current_tab,
            current_tab_id: tab_id,
            are_floating_panes_visible: floating_visible,
            tab_id_to_position,
            viewport_cols: 200,
            viewport_rows: 50,
        }
    }

    #[test]
    fn passive_pruning_without_registry_discards_closed_panes_and_history() {
        for state in ["absent", "exited", "held", "plugin"] {
            let mut manager = ScratchpadManager::new(make_configs(&["term"]));
            manager
                .passive_panes
                .insert("term".to_string(), HashMap::from([(10, 42)]));
            manager.passive_mru.insert(10, ("term".to_string(), 42, 0));
            manager.register_pane("term", 11, 99);
            let local_panes = manager.panes.clone();
            let local_history = manager.focus_times.clone();
            let positions = HashMap::from([(10, 0), (11, 1)]);
            let panes = if state == "absent" {
                Vec::new()
            } else {
                vec![PaneInfo {
                    id: 42,
                    exited: state == "exited",
                    is_held: state == "held",
                    is_plugin: state == "plugin",
                    ..Default::default()
                }]
            };
            let manifest = HashMap::from([(0, panes)]);
            let ctx = make_context(&manifest, 0, Some(10), true, &positions);
            // Registry unavailable: pruning and publication still run, discovery does not.
            manager.prune_passive_observations(&ctx);
            assert!(manager.passive_panes.is_empty(), "{state}");
            assert!(manager.passive_mru.is_empty(), "{state}");
            let (snapshot, tabs) = manager.publication_snapshot(&ctx);
            assert!(!snapshot.current_items[0].is_mru);
            assert!(!tabs[&10][0].is_mru);
            assert_eq!(tabs[&10][0].state, ScratchpadDisplayState::Closed);
            assert_eq!(manager.panes, local_panes);
            assert_eq!(manager.focus_times, local_history);
            assert_eq!(manager.just_shown, Some(99));
        }
    }

    #[test]
    fn passive_pruning_preserves_reorders_but_removes_closed_tabs_and_configs() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "notes"]));
        manager.passive_panes = HashMap::from([
            ("term".to_string(), HashMap::from([(10, 42), (11, 43)])),
            ("notes".to_string(), HashMap::from([(12, 44)])),
        ]);
        manager.passive_mru = HashMap::from([
            (10, ("term".to_string(), 42, 0)),
            (11, ("term".to_string(), 43, 0)),
            (12, ("notes".to_string(), 44, 0)),
        ]);
        let manifest = HashMap::from([(
            0,
            vec![
                make_floating_pane(42, false),
                make_floating_pane(43, false),
                make_floating_pane(44, false),
            ],
        )]);
        let positions = HashMap::from([(10, 7), (12, 2)]);
        manager.reconcile_config(make_configs(&["term"]));
        let ctx = make_context(&manifest, 7, Some(10), true, &positions);
        manager.prune_passive_observations(&ctx);
        assert_eq!(
            manager.passive_panes,
            HashMap::from([("term".to_string(), HashMap::from([(10, 42)]))])
        );
        assert_eq!(
            manager.passive_mru,
            HashMap::from([(10, ("term".to_string(), 42, 0))])
        );
        // Another unavailable read must not erase surviving observations/history.
        manager.prune_passive_observations(&ctx);
        assert_eq!(manager.passive_panes["term"][&10], 42);
        let positions = HashMap::new();
        manager.prune_passive_observations(&make_context(&manifest, 0, None, true, &positions));
        assert!(manager.passive_panes.is_empty());
        assert!(manager.passive_mru.is_empty());
    }

    #[test]
    fn passive_mru_survives_same_pane_sync_until_new_local_focus() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "notes"]));
        manager.register_pane("notes", 10, 43);
        manager.clear_just_shown();
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 10, 1, 0, 2_000);
        registry.finish_open("term", 10, 1, 42, 1);
        let positions = HashMap::from([(10, 0)]);
        let mut manifest = HashMap::from([(
            0,
            vec![make_floating_pane(42, true), make_floating_pane(43, false)],
        )]);
        let ctx = make_context(&manifest, 0, Some(10), true, &positions);
        manager.discover_panes_from_registry(&registry, &ctx);
        manager.sync_known_panes(&[("term".to_string(), 10, 42)]);
        manager.discover_panes_from_registry(&registry, &ctx);
        assert_eq!(manager.passive_mru[&10].0, "term");
        let pane = &mut manifest.get_mut(&0).unwrap()[0];
        pane.is_focused = false;
        pane.is_suppressed = true;
        let ctx = make_context(&manifest, 0, Some(10), true, &positions);
        manager.prune_passive_observations(&ctx);
        let (snapshot, tabs) = manager.publication_snapshot(&ctx);
        assert!(snapshot.current_items[1].is_mru);
        assert!(tabs[&10][1].is_mru);
        assert_eq!(
            manager.get_last_focused_on_current_tab(&ctx).as_deref(),
            Some("notes")
        );
        manager.register_pane("notes", 10, 43);
        manager.clear_just_shown();
        let (_, tabs) = manager.publication_snapshot(&ctx);
        assert!(tabs[&10][0].is_mru);
        assert!(!tabs[&10][1].is_mru);
    }

    #[test]
    fn passive_discovery_validates_observations_without_mutating_registry() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));
        let mut registry = ScratchpadRegistry::default();
        for tab_id in 10..18 {
            registry.begin_open("term", tab_id, 1, 0, 2_000);
            registry.finish_open("term", tab_id, 1, tab_id as u32, 1);
        }
        registry.entries[6].state = RegistryRecordState::Pending { owner_plugin_id: 1 };
        registry.entries[7].state = RegistryRecordState::Tombstone;
        registry.begin_open("unknown", 10, 1, 0, 2_000);
        registry.finish_open("unknown", 10, 1, 99, 1);
        let original = registry.clone();
        let positions = (10..17).map(|id| (id, id - 10)).collect();
        let mut manifest = HashMap::from([
            (
                0,
                vec![make_floating_pane(10, true), make_floating_pane(99, false)],
            ),
            (
                2,
                vec![PaneInfo {
                    id: 12,
                    exited: true,
                    ..Default::default()
                }],
            ),
            (
                3,
                vec![PaneInfo {
                    id: 13,
                    is_held: true,
                    ..Default::default()
                }],
            ),
            (
                4,
                vec![PaneInfo {
                    id: 14,
                    is_plugin: true,
                    ..Default::default()
                }],
            ),
            (5, vec![make_floating_pane(16, false)]),
            (6, vec![make_floating_pane(15, false)]),
            (7, vec![make_floating_pane(17, false)]),
        ]);
        manager.discover_panes_from_registry(
            &registry,
            &make_context(&manifest, 0, Some(10), true, &positions),
        );
        assert_eq!(
            manager.passive_panes,
            HashMap::from([("term".to_string(), HashMap::from([(10, 10)]))])
        );
        assert_eq!(registry, original);

        // A later snapshot can discover a pane without any registry reconciliation.
        manifest.insert(1, vec![make_floating_pane(11, false)]);
        manager.discover_panes_from_registry(
            &registry,
            &make_context(&manifest, 0, Some(10), true, &positions),
        );
        assert_eq!(
            manager.passive_panes["term"],
            HashMap::from([(10, 10), (11, 11)])
        );
        assert_eq!(registry, original);
        assert!(manager.panes.is_empty());
        assert!(manager.focus_times.is_empty());
        assert_eq!(manager.just_shown, None);

        // A confirmed replacement on the same key must not stay closed forever.
        registry.entries[1].state = RegistryRecordState::Present { pane_id: 111 };
        manifest.insert(1, vec![make_floating_pane(111, false)]);
        let ctx = make_context(&manifest, 0, Some(10), true, &positions);
        manager.discover_panes_from_registry(&registry, &ctx);
        assert_eq!(
            manager.publication_snapshot(&ctx).1[&11][0].pane_id,
            Some(111)
        );
        assert!(manager.panes.is_empty());
    }

    #[test]
    fn passive_discovery_preserves_local_state_and_never_transfers_panes() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "notes"]));
        manager.register_pane("term", 10, 42);
        manager.register_pane("notes", 12, 99);
        manager
            .orphaned
            .insert("old".to_string(), HashSet::from([13]));
        let panes = manager.panes.clone();
        let focus_times = manager.focus_times.clone();
        let orphaned = manager.orphaned.clone();
        let focus_counter = manager.focus_counter;
        let just_shown = manager.just_shown;
        let mut registry = ScratchpadRegistry::default();
        for (name, tab_id, pane_id) in [("term", 11, 42), ("notes", 11, 42), ("notes", 12, 100)] {
            registry.begin_open(name, tab_id, 1, 0, 2_000);
            registry.finish_open(name, tab_id, 1, pane_id, 1);
        }
        let manifest = HashMap::from([
            (1, vec![make_floating_pane(42, true)]),
            (2, vec![make_floating_pane(100, false)]),
        ]);
        let positions = HashMap::from([(10, 0), (11, 1), (12, 2)]);
        let ctx = make_context(&manifest, 0, Some(10), true, &positions);
        manager.discover_panes_from_registry(&registry, &ctx);
        manager.discover_panes_from_registry(&ScratchpadRegistry::default(), &ctx);
        assert_eq!(manager.panes, panes);
        assert_eq!(manager.focus_times, focus_times);
        assert_eq!(manager.orphaned, orphaned);
        assert_eq!(manager.focus_counter, focus_counter);
        assert_eq!(manager.just_shown, just_shown);
        let (_, tabs) = manager.publication_snapshot(&ctx);
        assert!(tabs[&11]
            .iter()
            .all(|item| item.state == ScratchpadDisplayState::Closed));
    }

    #[test]
    fn passive_panes_are_never_action_owned_or_cleaned_up_as_orphans() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 10, 1, 0, 2_000);
        registry.finish_open("term", 10, 1, 42, 1);
        let manifest = HashMap::from([(0, vec![make_floating_pane(42, true)])]);
        let positions = HashMap::from([(10, 0)]);
        let ctx = make_context(&manifest, 0, Some(10), true, &positions);
        manager.discover_panes_from_registry(&registry, &ctx);
        let (snapshot, tabs) = manager.publication_snapshot(&ctx);
        assert_eq!(snapshot.current_items[0].pane_id, Some(42));
        assert_eq!(snapshot.global_items[0].pane_id, Some(42));
        assert_eq!(snapshot.global_count_items[0].pane_id, Some(42));
        assert_eq!(tabs[&10][0].pane_id, Some(42));
        assert!(manager.persisted_state().panes.is_empty());
        assert_eq!(
            manager.status_snapshot(&ctx).current_items[0].state,
            ScratchpadDisplayState::Closed
        );
        assert!(manager.handle_focus_pane(42, &ctx).is_none());
        assert!(manager.handle_toggle(None, &ctx).is_empty());
        assert!(manager.handle_hide("term", &ctx).is_empty());
        assert!(manager.handle_close("term", &ctx).is_empty());
        assert!(matches!(
            manager.handle_show("term", &ctx).as_slice(),
            [ScratchpadCommand::OpenFloating { .. }]
        ));

        // A stale position-derived orphan set must never close another client's pane.
        assert!(manager
            .on_pane_update(&ctx, &HashSet::from([10]))
            .is_empty());
        assert!(manager.focus_times.is_empty());
        assert_eq!(manager.focus_counter, 0);
        assert!(manager.reconcile_config(HashMap::new()).is_empty());
        assert!(manager.orphaned.is_empty());
        manager.reconcile_config(make_configs(&["term"]));
        assert_eq!(
            manager.publication_snapshot(&ctx).1[&10][0].pane_id,
            Some(42)
        );
    }

    #[test]
    fn passive_first_focus_survives_hide_without_affecting_action_mru() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "notes"]));
        manager.register_pane("notes", 10, 43);
        manager.clear_just_shown();
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 10, 1, 0, 2_000);
        registry.finish_open("term", 10, 1, 42, 1);
        let positions = HashMap::from([(10, 0)]);
        let mut manifest = HashMap::from([(
            0,
            vec![make_floating_pane(42, true), make_floating_pane(43, false)],
        )]);
        manager.discover_panes_from_registry(
            &registry,
            &make_context(&manifest, 0, Some(10), true, &positions),
        );
        assert_eq!(manager.passive_mru[&10].0, "term");
        let pane = &mut manifest.get_mut(&0).unwrap()[0];
        pane.is_focused = false;
        pane.is_suppressed = true;
        let ctx = make_context(&manifest, 0, Some(10), true, &positions);
        // Empty reads preserve both observations and passive focus history.
        manager.discover_panes_from_registry(&ScratchpadRegistry::default(), &ctx);
        let (snapshot, tabs) = manager.publication_snapshot(&ctx);
        assert!(tabs[&10][1].is_mru);
        assert_eq!(tabs[&10][1].state, ScratchpadDisplayState::Hidden);
        assert!(snapshot.current_items[1].is_mru);
        assert_eq!(
            manager.get_last_focused_on_current_tab(&ctx).as_deref(),
            Some("notes")
        );
        assert!(manager.status_snapshot(&ctx).current_items[0].is_mru);
        assert_eq!(manager.focus_times.len(), 1);
        assert_eq!(manager.focus_counter, 1);

        // A later local focus wins without using the passive counter for actions.
        manager.handle_show("notes", &ctx);
        manager.clear_just_shown();
        let (_, tabs) = manager.publication_snapshot(&ctx);
        assert!(tabs[&10][0].is_mru);
        assert!(!tabs[&10][1].is_mru);
    }

    #[test]
    fn passive_identity_cannot_transfer_and_later_local_ownership_wins() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "notes"]));
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 10, 1, 0, 2_000);
        registry.finish_open("term", 10, 1, 42, 1);
        let positions = HashMap::from([(10, 0), (11, 1)]);
        let manifest = HashMap::from([(0, vec![make_floating_pane(42, true)])]);
        manager.discover_panes_from_registry(
            &registry,
            &make_context(&manifest, 0, Some(10), true, &positions),
        );
        let passive = manager.passive_panes.clone();
        registry.entries[0].name = "notes".to_string();
        registry.entries[0].tab_id = 11;
        let manifest = HashMap::from([(1, vec![make_floating_pane(42, true)])]);
        let ctx = make_context(&manifest, 1, Some(11), true, &positions);
        manager.discover_panes_from_registry(&registry, &ctx);
        assert_eq!(manager.passive_panes, passive);
        assert!(manager
            .publication_snapshot(&ctx)
            .1
            .values()
            .flatten()
            .all(|item| item.pane_id.is_none()));

        // Explicit action ownership, not discovery, can claim the pane elsewhere.
        manager.register_pane("notes", 11, 42);
        let (_, tabs) = manager.publication_snapshot(&ctx);
        assert_eq!(tabs[&11][0].pane_id, Some(42));
        assert_eq!(tabs[&10][1].pane_id, None);

        // An in-flight local pane also masks the same passive name/tab key.
        manager.register_pane("term", 10, 99);
        let manifest = HashMap::from([(0, vec![make_floating_pane(42, true)])]);
        let ctx = make_context(&manifest, 0, Some(10), true, &positions);
        assert_eq!(manager.publication_snapshot(&ctx).1[&10][1].pane_id, None);
    }

    #[test]
    fn show_unconfigured_scratchpad_is_noop() {
        let mut manager = ScratchpadManager::new(HashMap::new());
        let manifest = HashMap::new();
        let positions = HashMap::new();
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        let commands = manager.handle_action(
            ScratchpadAction::Show {
                name: "unknown".to_string(),
                target: Default::default(),
            },
            &ctx,
        );
        assert!(commands.is_empty());
    }

    #[test]
    fn show_new_scratchpad_opens_floating() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));
        let manifest = HashMap::new();
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        let commands = manager.handle_action(
            ScratchpadAction::Show {
                name: "term".to_string(),
                target: Default::default(),
            },
            &ctx,
        );

        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            ScratchpadCommand::OpenFloating { .. }
        ));
    }

    #[test]
    fn show_existing_scratchpad_shows_pane() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        // Register the scratchpad first
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.register_pane("term", 0, 42);

        // Now show it
        let commands = manager.handle_action(
            ScratchpadAction::Show {
                name: "term".to_string(),
                target: Default::default(),
            },
            &ctx,
        );

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::ShowPane { pane_id: 42, .. })));
    }

    #[test]
    fn hide_scratchpad_hides_pane() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.register_pane("term", 0, 42);

        let commands = manager.handle_action(
            ScratchpadAction::Hide {
                name: "term".to_string(),
                target: Default::default(),
            },
            &ctx,
        );

        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            ScratchpadCommand::HidePane { pane_id: 42 }
        ));
    }

    #[test]
    fn close_scratchpad_closes_pane() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.register_pane("term", 0, 42);

        let commands = manager.handle_action(
            ScratchpadAction::Close {
                name: "term".to_string(),
                target: Default::default(),
            },
            &ctx,
        );

        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            ScratchpadCommand::ClosePane { pane_id: 42 }
        ));
    }

    #[test]
    fn toggle_hidden_scratchpad_shows_it() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        // Register with suppressed (hidden) pane
        let mut manifest = HashMap::new();
        let mut pane = make_floating_pane(42, false);
        pane.is_suppressed = true;
        manifest.insert(0, vec![pane]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.register_pane("term", 0, 42);
        manager.clear_just_shown(); // Simulate PaneUpdate

        let commands = manager.handle_action(
            ScratchpadAction::Toggle {
                name: Some("term".to_string()),
                target: Default::default(),
            },
            &ctx,
        );

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::ShowPane { pane_id: 42, .. })));
    }

    #[test]
    fn toggle_visible_focused_scratchpad_hides_it() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.register_pane("term", 0, 42);

        let commands = manager.handle_action(
            ScratchpadAction::Toggle {
                name: Some("term".to_string()),
                target: Default::default(),
            },
            &ctx,
        );

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::HidePane { pane_id: 42 })));
    }

    #[test]
    fn synced_external_scratchpad_toggle_hides_without_local_open() {
        // Reproduces the multi-client repro: this instance never opened "term"
        // (another client's instance did). After syncing the shared registry
        // state, the first toggle must hide, not re-show.
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));
        manager.sync_known_panes(&[("term".to_string(), 0, 42)]);

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        let commands = manager.handle_action(
            ScratchpadAction::Toggle {
                name: Some("term".to_string()),
                target: Default::default(),
            },
            &ctx,
        );

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::HidePane { pane_id: 42 })));
    }

    #[test]
    fn toggle_hides_visible_focused_scratchpad_despite_stale_tab_flag() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        // Simulate a stale TabUpdate: `are_floating_panes_visible` is false while
        // the PaneUpdate manifest already shows the scratchpad floating + focused.
        // The decision must come from the manifest, so this should still hide.
        let ctx = make_context(&manifest, 0, Some(0), false, &positions);

        manager.register_pane("term", 0, 42);
        manager.clear_just_shown(); // force the decision to read the manifest

        let commands = manager.handle_action(
            ScratchpadAction::Toggle {
                name: Some("term".to_string()),
                target: Default::default(),
            },
            &ctx,
        );

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::HidePane { pane_id: 42 })));
    }

    #[test]
    fn unnamed_toggle_and_status_use_each_tabs_mru_not_other_tabs_focus() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "notes"]));
        manager.register_pane("term", 10, 42);
        manager.register_pane("notes", 11, 43);
        manager.clear_just_shown();
        let positions = HashMap::from([(10, 0), (11, 1)]);

        for (tab_id, position, name, pane_id, other_pane_id) in
            [(10, 0, "term", 42, 43), (11, 1, "notes", 43, 42)]
        {
            for floating in [false, true] {
                let mut hidden = make_floating_pane(pane_id, false);
                hidden.is_suppressed = true;
                let manifest = HashMap::from([
                    (position, vec![hidden, make_pane(99, floating, true)]),
                    (1 - position, vec![make_floating_pane(other_pane_id, true)]),
                ]);
                let ctx = make_context(&manifest, position, Some(tab_id), false, &positions);
                let history = manager.focus_times.clone();
                manager.update_focus_tracking(&ctx);
                assert_eq!(manager.focus_times, history);

                let snapshot = manager.status_snapshot(&ctx);
                let mru: Vec<_> = snapshot
                    .current_items
                    .iter()
                    .filter(|item| item.is_mru)
                    .collect();
                assert_eq!(mru.len(), 1);
                assert_eq!(mru[0].name, name);
                let commands = manager.handle_toggle(None, &ctx);
                assert!(matches!(
                    commands.as_slice(),
                    [ScratchpadCommand::ShowPane { pane_id: id, tab_id: tab, .. }]
                        if *id == pane_id && *tab == tab_id
                ));
                manager.clear_just_shown();
            }
        }

        let manifest = HashMap::from([(1, vec![make_floating_pane(43, true)])]);
        let ctx = make_context(&manifest, 2, Some(12), true, &positions);
        assert!(manager.handle_toggle(None, &ctx).is_empty());
        assert!(manager
            .status_snapshot(&ctx)
            .current_items
            .iter()
            .all(|item| !item.is_mru));
    }

    #[test]
    fn unnamed_toggle_hides_current_focus_and_remembers_it_after_hiding() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "notes"]));
        manager.register_pane("term", 10, 42);
        manager.register_pane("notes", 10, 43);
        manager.register_pane("notes", 11, 44);
        manager.clear_just_shown();
        let positions = HashMap::from([(10, 0), (11, 1)]);
        let mut manifest = HashMap::from([
            (
                0,
                vec![make_floating_pane(42, true), make_floating_pane(43, false)],
            ),
            (1, vec![make_floating_pane(44, true)]),
        ]);
        let ctx = make_context(&manifest, 0, Some(10), false, &positions);
        assert_eq!(
            manager.get_focused_scratchpad(&ctx).as_deref(),
            Some("term")
        );
        assert!(matches!(
            manager.handle_toggle(None, &ctx).as_slice(),
            [ScratchpadCommand::HidePane { pane_id: 42 }]
        ));
        manager.update_focus_tracking(&ctx);
        assert_eq!(manager.focus_times["notes"][&11], 3);
        assert_eq!(
            manager.get_last_focused_on_current_tab(&ctx).as_deref(),
            Some("term")
        );

        let pane = &mut manifest.get_mut(&0).unwrap()[0];
        pane.is_focused = false;
        pane.is_suppressed = true;
        let ctx = make_context(&manifest, 0, Some(10), true, &positions);
        assert!(matches!(
            manager.handle_toggle(None, &ctx).as_slice(),
            [ScratchpadCommand::ShowPane {
                pane_id: 42,
                tab_id: 10,
                ..
            }]
        ));
        // A second toggle before PaneUpdate must hide the pane just shown.
        assert!(matches!(
            manager.handle_toggle(None, &ctx).as_slice(),
            [ScratchpadCommand::HidePane { pane_id: 42 }]
        ));
    }

    #[test]
    fn on_pane_update_closes_exited_scratchpads() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        // Register scratchpad
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);

        manager.register_pane("term", 0, 42);

        // Pane has exited
        let mut exited_pane = make_floating_pane(42, false);
        exited_pane.exited = true;
        manifest.insert(0, vec![exited_pane]);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        let commands = manager.on_pane_update(&ctx, &HashSet::new());

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::ClosePane { pane_id: 42 })));
    }

    #[test]
    fn on_pane_update_closes_orphaned_tab_scratchpads() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.register_pane("term", 0, 42);

        // Tab 0 is now orphaned
        let mut orphaned = HashSet::new();
        orphaned.insert(0);

        let commands = manager.on_pane_update(&ctx, &orphaned);

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::ClosePane { pane_id: 42 })));
    }

    #[test]
    fn persisted_state_roundtrip() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "htop"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        manifest.insert(1, vec![make_floating_pane(99, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        positions.insert(1, 1);

        // Register two scratchpads
        manager.register_pane("term", 0, 42);
        manager.register_pane("htop", 1, 99);

        // Get persisted state
        let state = manager.persisted_state();
        assert_eq!(state.panes.len(), 2);
        assert_eq!(state.focus_times.len(), 2);
        assert!(state.focus_counter >= 2);

        // Create new manager and restore
        let mut new_manager = ScratchpadManager::new(make_configs(&["term", "htop"]));
        let commands = new_manager.restore_state(state);

        // No orphans since configs match
        assert!(commands.is_empty());

        // Verify state was restored
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);
        assert!(new_manager.get_pane("term", &ctx).is_some());
    }

    #[test]
    fn restore_state_detects_orphans() {
        // Create a manager with only "term" config, but state has "htop" pane
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let state = PersistedState {
            panes: vec![
                (("term".to_string(), 0), 42),
                (("htop".to_string(), 0), 99), // This will be orphaned
            ],
            focus_times: vec![(("term".to_string(), 0), 1), (("htop".to_string(), 0), 2)],
            focus_counter: 2,
        };

        let commands = manager.restore_state(state);

        // Should have command to show orphaned htop pane
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            ScratchpadCommand::ShowPane { pane_id: 99, .. }
        ));
    }

    #[test]
    fn reconcile_config_detects_orphans() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "htop"]));

        let mut manifest = HashMap::new();
        manifest.insert(
            0,
            vec![make_floating_pane(42, true), make_floating_pane(99, false)],
        );
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        // Register both
        manager.register_pane("term", 0, 42);
        manager.register_pane("htop", 0, 99);

        // Reconcile with config that only has "term"
        let commands = manager.reconcile_config(make_configs(&["term"]));

        // Should show htop pane since it was removed from config
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            ScratchpadCommand::ShowPane { pane_id: 99, .. }
        ));

        // Toggle on htop should be a no-op now
        let commands = manager.handle_action(
            ScratchpadAction::Toggle {
                name: Some("htop".to_string()),
                target: Default::default(),
            },
            &ctx,
        );
        assert!(commands.is_empty());
    }

    #[test]
    fn reconcile_config_unorphans_readded_scratchpad() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "htop"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(99, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        // Register htop
        manager.register_pane("htop", 0, 99);

        // Remove htop from config (orphan it)
        manager.reconcile_config(make_configs(&["term"]));

        // Re-add htop to config
        manager.reconcile_config(make_configs(&["term", "htop"]));

        // Toggle on htop should work again
        let commands = manager.handle_action(
            ScratchpadAction::Toggle {
                name: Some("htop".to_string()),
                target: Default::default(),
            },
            &ctx,
        );
        // Should try to hide since it's visible and focused
        assert!(!commands.is_empty());
    }

    #[test]
    fn on_pane_update_cleans_up_missing_panes_silently() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        // Register scratchpad with pane 42
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);

        manager.register_pane("term", 0, 42);

        // Pane 42 is gone from manifest entirely (closed externally)
        let empty_manifest: HashMap<usize, Vec<PaneInfo>> = HashMap::new();
        let ctx = make_context(&empty_manifest, 0, Some(0), true, &positions);

        let commands = manager.on_pane_update(&ctx, &HashSet::new());

        // No ClosePane command should be emitted (pane is already gone)
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, ScratchpadCommand::ClosePane { .. })),
            "Should not emit ClosePane for panes already missing from manifest"
        );

        // The pane should be removed from tracking
        assert!(
            manager.get_pane("term", &ctx).is_none(),
            "Pane should be removed from tracking after cleanup"
        );
    }

    #[test]
    fn on_pane_update_closes_held_scratchpads() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        // Register scratchpad
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);

        manager.register_pane("term", 0, 42);

        // Pane is now held (e.g. command exited with exit-on-close disabled)
        let mut held_pane = make_floating_pane(42, false);
        held_pane.is_held = true;
        manifest.insert(0, vec![held_pane]);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        let commands = manager.on_pane_update(&ctx, &HashSet::new());

        assert!(
            commands
                .iter()
                .any(|c| matches!(c, ScratchpadCommand::ClosePane { pane_id: 42 })),
            "Should emit ClosePane for held panes"
        );

        // Pane should be removed from tracking
        assert!(
            manager.get_pane("term", &ctx).is_none(),
            "Held pane should be removed from tracking"
        );
    }

    #[test]
    fn register_with_title_emits_rename() {
        let mut config = make_config("bash");
        config.title = Some("My Shell".to_string());
        let configs = HashMap::from([("term".to_string(), config)]);
        let mut manager = ScratchpadManager::new(configs);

        let commands = manager.register_pane("term", 0, 42);

        assert!(
            commands.iter().any(
                |c| matches!(c, ScratchpadCommand::RenamePane { pane_id: 42, name } if name == "My Shell")
            ),
            "Should emit RenamePane with the configured title"
        );
    }

    #[test]
    fn register_without_title_uses_scratchpad_name() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let commands = manager.register_pane("term", 0, 42);

        assert!(
            commands.iter().any(
                |c| matches!(c, ScratchpadCommand::RenamePane { pane_id: 42, name } if name == "term")
            ),
            "Should emit RenamePane with the scratchpad name as fallback"
        );
    }

    #[test]
    fn register_with_cwd_sets_cwd_on_command() {
        let mut config = make_config("bash");
        config.cwd = Some("/tmp/work".to_string());
        let configs = HashMap::from([("term".to_string(), config)]);
        let mut manager = ScratchpadManager::new(configs);

        let manifest = HashMap::new();
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        let commands = manager.handle_action(
            ScratchpadAction::Show {
                name: "term".to_string(),
                target: Default::default(),
            },
            &ctx,
        );

        assert_eq!(commands.len(), 1);
        match &commands[0] {
            ScratchpadCommand::OpenFloating { command, .. } => {
                assert_eq!(
                    command.cwd.as_deref(),
                    Some(std::path::Path::new("/tmp/work")),
                    "CommandToRun should have cwd set"
                );
            }
            other => panic!("Expected OpenFloating, got {:?}", other),
        }
    }

    #[test]
    fn focus_pane_for_scratchpad_returns_show_commands() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        // Register the scratchpad
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.register_pane("term", 0, 42);
        manager.clear_just_shown();

        // focus-pane for a scratchpad should return show commands
        let result = manager.handle_focus_pane(42, &ctx);
        assert!(
            result.is_some(),
            "Should return commands for a scratchpad pane"
        );

        let commands = result.unwrap();
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, ScratchpadCommand::ShowPane { pane_id: 42, .. })),
            "Should emit ShowPane with coordinates for the scratchpad"
        );
    }

    #[test]
    fn focus_pane_for_non_scratchpad_returns_none() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));
        let manifest = HashMap::new();
        let positions = HashMap::new();
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        // Pane 99 is not a scratchpad
        let result = manager.handle_focus_pane(99, &ctx);
        assert!(
            result.is_none(),
            "Should return None for non-scratchpad pane"
        );
    }

    #[test]
    fn focus_pane_for_orphaned_scratchpad_returns_none() {
        let mut manager = ScratchpadManager::new(make_configs(&["term", "htop"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(99, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        // Register htop
        manager.register_pane("htop", 0, 99);

        // Remove htop from config (orphan it)
        manager.reconcile_config(make_configs(&["term"]));

        // focus-pane for an orphaned scratchpad should return None
        let result = manager.handle_focus_pane(99, &ctx);
        assert!(
            result.is_none(),
            "Should return None for orphaned scratchpad pane"
        );
    }

    #[test]
    fn focus_pane_scratchpad_on_different_tab() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        // Register scratchpad on tab 0 (tab id 0)
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        manifest.insert(1, vec![]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        positions.insert(1, 1);

        manager.register_pane("term", 0, 42);
        manager.clear_just_shown();

        // Now we're on tab 1, but focusing a scratchpad on tab 0
        let ctx = make_context(&manifest, 1, Some(1), true, &positions);
        let result = manager.handle_focus_pane(42, &ctx);
        assert!(
            result.is_some(),
            "Should return commands even when scratchpad is on a different tab"
        );

        let commands = result.unwrap();
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, ScratchpadCommand::ShowPane { pane_id: 42, .. })),
            "Should emit ShowPane for cross-tab scratchpad focus"
        );
    }
}
