use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};

use zellij_tile::prelude::*;

/// A stable tab identifier that doesn't change when tabs are reordered.
/// We use a monotonically increasing counter assigned when we first see a tab.
type StableTabId = u64;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ScratchpadScope {
    #[default]
    Tab,
    Session,
}

#[derive(Debug, Clone, Deserialize)]
struct ScratchpadConfig {
    command: Vec<String>,
    #[serde(default)]
    scope: ScratchpadScope,
}

#[derive(Debug)]
enum ScratchpadAction {
    Toggle { name: Option<String> },
    Show { name: String },
    Hide { name: String },
    Close { name: String },
    RegisterTab { name: String, pane_id: u32 },
    RegisterSession { name: String, pane_id: u32 },
}

#[derive(Default)]
struct State {
    // Pane tracking (from PaneUpdate events)
    // Key is tab position (0-indexed), which changes when tabs are reordered
    pane_manifest: HashMap<usize, Vec<PaneInfo>>,

    // Current tab position and floating pane visibility (from TabUpdate events)
    current_tab_position: usize,
    are_floating_panes_visible: bool,

    // Stable tab ID tracking:
    // - We assign each tab a stable ID when we first see it
    // - We identify tabs by finding a "reference pane" (any tiled pane) on that tab
    // - When tabs are reordered, we can find the same tab by looking for its reference pane
    next_stable_tab_id: StableTabId,
    // reference_pane_id -> stable_tab_id (tiled panes that identify tabs)
    reference_pane_to_tab: HashMap<u32, StableTabId>,
    // stable_tab_id -> current position (updated on each PaneUpdate)
    stable_tab_to_position: HashMap<StableTabId, usize>,

    // Scratchpad configuration (from plugin load)
    scratchpad_configs: HashMap<String, ScratchpadConfig>,

    // Tab-scoped scratchpad state: (name, stable_tab_id) -> pane_id
    tab_scratchpad_panes: HashMap<(String, StableTabId), u32>,
    // Pending registrations: (name, stable_tab_id)
    tab_pending_registrations: HashSet<(String, StableTabId)>,

    // Session-scoped scratchpad state: name -> pane_id
    session_scratchpad_panes: HashMap<String, u32>,
    session_pending_registrations: HashSet<String>,

    // Focus history: (name, stable_tab_id) -> last focus timestamp
    focus_counter: u64,
    scratchpad_focus_times: HashMap<(String, StableTabId), u64>,

    // Track scratchpads we've just shown/focused (cleared on next PaneUpdate)
    // This handles the race condition where we show a pane but haven't received
    // the PaneUpdate with the updated is_focused state yet.
    just_shown_scratchpad: Option<u32>, // pane_id
}

register_plugin!(State);

#[derive(Debug)]
enum ParseError {
    InvalidFormat,
    WrongPlugin,
    UnknownEvent(String),
    InvalidArgs(String),
    InvalidScratchpadName(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::InvalidFormat => {
                write!(f, "Message format should be 'plugin::event::args...'")
            }
            ParseError::WrongPlugin => write!(f, "Message not intended for zellij-tools"),
            ParseError::UnknownEvent(event) => write!(f, "Unknown event: {}", event),
            ParseError::InvalidArgs(msg) => write!(f, "Invalid arguments: {}", msg),
            ParseError::InvalidScratchpadName(name) => {
                write!(
                    f,
                    "Invalid scratchpad name '{}': must match [a-zA-Z0-9_-]+",
                    name
                )
            }
        }
    }
}

fn is_valid_scratchpad_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

impl State {
    fn parse_message(&self, payload: &str) -> Result<(String, Vec<String>), ParseError> {
        let mut parts = payload.splitn(3, "::");

        let plugin = parts.next().ok_or(ParseError::InvalidFormat)?;
        let event = parts.next().ok_or(ParseError::InvalidFormat)?;
        let args_str = parts.next().unwrap_or("");

        if plugin != "zellij-tools" {
            return Err(ParseError::WrongPlugin);
        }

        let args: Vec<String> = if args_str.is_empty() {
            Vec::new()
        } else {
            args_str.split("::").map(|s| s.to_string()).collect()
        };

        Ok((event.to_string(), args))
    }

    fn parse_scratchpad_action(&self, args: &[String]) -> Result<ScratchpadAction, ParseError> {
        let action = args.first().map(|s| s.as_str()).unwrap_or("");

        match action {
            "toggle" => {
                let name = args.get(1).cloned();
                if let Some(ref n) = name {
                    if !is_valid_scratchpad_name(n) {
                        return Err(ParseError::InvalidScratchpadName(n.clone()));
                    }
                }
                Ok(ScratchpadAction::Toggle { name })
            }
            "show" => {
                let name = args.get(1).ok_or_else(|| {
                    ParseError::InvalidArgs("show requires a scratchpad name".to_string())
                })?;
                if !is_valid_scratchpad_name(name) {
                    return Err(ParseError::InvalidScratchpadName(name.clone()));
                }
                Ok(ScratchpadAction::Show { name: name.clone() })
            }
            "hide" => {
                let name = args.get(1).ok_or_else(|| {
                    ParseError::InvalidArgs("hide requires a scratchpad name".to_string())
                })?;
                if !is_valid_scratchpad_name(name) {
                    return Err(ParseError::InvalidScratchpadName(name.clone()));
                }
                Ok(ScratchpadAction::Hide { name: name.clone() })
            }
            "close" => {
                let name = args.get(1).ok_or_else(|| {
                    ParseError::InvalidArgs("close requires a scratchpad name".to_string())
                })?;
                if !is_valid_scratchpad_name(name) {
                    return Err(ParseError::InvalidScratchpadName(name.clone()));
                }
                Ok(ScratchpadAction::Close { name: name.clone() })
            }
            "register" => {
                // Format: register::session::<name>::<pane_id>
                // Format: register::tab::<name>::<pane_id>
                let scope = args.get(1).ok_or_else(|| {
                    ParseError::InvalidArgs(
                        "register requires a scope (session or tab)".to_string(),
                    )
                })?;

                match scope.as_str() {
                    "session" => {
                        let name = args.get(2).ok_or_else(|| {
                            ParseError::InvalidArgs("register::session requires a name".to_string())
                        })?;
                        let pane_id_str = args.get(3).ok_or_else(|| {
                            ParseError::InvalidArgs(
                                "register::session requires a pane_id".to_string(),
                            )
                        })?;
                        let pane_id = pane_id_str.parse::<u32>().map_err(|e| {
                            ParseError::InvalidArgs(format!(
                                "Invalid pane_id '{}': {}",
                                pane_id_str, e
                            ))
                        })?;
                        if !is_valid_scratchpad_name(name) {
                            return Err(ParseError::InvalidScratchpadName(name.clone()));
                        }
                        Ok(ScratchpadAction::RegisterSession {
                            name: name.clone(),
                            pane_id,
                        })
                    }
                    "tab" => {
                        let name = args.get(2).ok_or_else(|| {
                            ParseError::InvalidArgs("register::tab requires a name".to_string())
                        })?;
                        let pane_id_str = args.get(3).ok_or_else(|| {
                            ParseError::InvalidArgs("register::tab requires a pane_id".to_string())
                        })?;
                        let pane_id = pane_id_str.parse::<u32>().map_err(|e| {
                            ParseError::InvalidArgs(format!(
                                "Invalid pane_id '{}': {}",
                                pane_id_str, e
                            ))
                        })?;
                        if !is_valid_scratchpad_name(name) {
                            return Err(ParseError::InvalidScratchpadName(name.clone()));
                        }
                        Ok(ScratchpadAction::RegisterTab {
                            name: name.clone(),
                            pane_id,
                        })
                    }
                    _ => Err(ParseError::InvalidArgs(format!(
                        "Unknown register scope: {}",
                        scope
                    ))),
                }
            }
            _ => Err(ParseError::InvalidArgs(format!(
                "Unknown scratchpad action: {}",
                action
            ))),
        }
    }

    /// Update stable tab ID mappings based on current pane_manifest.
    /// This identifies tabs by finding reference panes (tiled panes) that we've seen before.
    ///
    /// Key insight: A reference pane keeps its stable ID even if it moves to a different
    /// tab position. The stable ID follows the reference pane, not the position.
    ///
    /// The algorithm handles these edge cases:
    /// 1. Reference pane moved to different tab position -> stable ID follows the pane
    /// 2. Reference pane closed -> pick new reference pane from remaining tiled panes ON SAME TAB
    /// 3. Tab closed entirely -> stable ID becomes orphaned, scratchpads keyed to it are stale
    fn update_stable_tab_mapping(&mut self) {
        // Build a map of pane_id -> current tab position for all tiled panes
        let pane_to_current_tab: HashMap<u32, usize> = self
            .pane_manifest
            .iter()
            .flat_map(|(&tab_pos, panes)| {
                panes
                    .iter()
                    .filter(|p| !p.is_floating && !p.is_plugin)
                    .map(move |p| (p.id, tab_pos))
            })
            .collect();

        // Clean up reference panes that no longer exist (were closed)
        let existing_pane_ids: HashSet<u32> = pane_to_current_tab.keys().copied().collect();
        let closed_refs: Vec<(u32, StableTabId)> = self
            .reference_pane_to_tab
            .iter()
            .filter(|(pane_id, _)| !existing_pane_ids.contains(pane_id))
            .map(|(&pane_id, &stable_id)| (pane_id, stable_id))
            .collect();

        for (pane_id, _stable_id) in &closed_refs {
            self.reference_pane_to_tab.remove(pane_id);
        }

        // Rebuild stable_tab_to_position based on where reference panes are NOW
        // (stable ID follows the reference pane, even if position changed)
        self.stable_tab_to_position.clear();
        for (&pane_id, &stable_id) in &self.reference_pane_to_tab {
            if let Some(&tab_position) = pane_to_current_tab.get(&pane_id) {
                self.stable_tab_to_position.insert(stable_id, tab_position);
            }
        }

        // For closed reference panes, try to find a replacement on the same stable tab
        // We need to find another tiled pane that's on the same position the stable ID was at
        for (_closed_pane_id, stable_id) in &closed_refs {
            // Skip if this stable ID already got reassigned (shouldn't happen but be safe)
            if self.stable_tab_to_position.contains_key(stable_id) {
                continue;
            }

            // Find tabs that don't have a stable ID yet
            for (&tab_position, panes) in &self.pane_manifest {
                // Skip if this position already has a stable ID
                if self
                    .stable_tab_to_position
                    .values()
                    .any(|&pos| pos == tab_position)
                {
                    continue;
                }

                // Get tiled panes on this tab
                let tiled_panes: Vec<u32> = panes
                    .iter()
                    .filter(|p| !p.is_floating && !p.is_plugin)
                    .map(|p| p.id)
                    .collect();

                if let Some(&new_ref_pane) = tiled_panes.first() {
                    // Assign this orphaned stable ID to this tab with a new reference pane
                    self.reference_pane_to_tab.insert(new_ref_pane, *stable_id);
                    self.stable_tab_to_position.insert(*stable_id, tab_position);
                    break; // Only reassign to one tab
                }
            }
        }

        // Finally, assign new stable IDs to any tabs that still don't have one
        for (&tab_position, panes) in &self.pane_manifest {
            // Skip if this position already has a stable ID
            if self
                .stable_tab_to_position
                .values()
                .any(|&pos| pos == tab_position)
            {
                continue;
            }

            // Get tiled panes on this tab
            let tiled_panes: Vec<u32> = panes
                .iter()
                .filter(|p| !p.is_floating && !p.is_plugin)
                .map(|p| p.id)
                .collect();

            if tiled_panes.is_empty() {
                continue;
            }

            let new_id = self.next_stable_tab_id;
            self.next_stable_tab_id += 1;
            self.reference_pane_to_tab.insert(tiled_panes[0], new_id);
            self.stable_tab_to_position.insert(new_id, tab_position);
        }
    }

    /// Get the stable tab ID for the current tab position
    fn get_current_stable_tab_id(&self) -> Option<StableTabId> {
        self.stable_tab_to_position
            .iter()
            .find(|(_, &pos)| pos == self.current_tab_position)
            .map(|(&id, _)| id)
    }

    fn cleanup_closed_scratchpads(&mut self) {
        // Collect all pane IDs that still exist in the manifest
        let existing_pane_ids: HashSet<u32> = self
            .pane_manifest
            .values()
            .flatten()
            .map(|p| p.id)
            .collect();

        // Clean up tab-scoped scratchpads whose panes no longer exist
        let closed_tab_keys: Vec<(String, StableTabId)> = self
            .tab_scratchpad_panes
            .iter()
            .filter(|(_, &pane_id)| !existing_pane_ids.contains(&pane_id))
            .map(|(key, _)| key.clone())
            .collect();

        for key in closed_tab_keys {
            self.tab_scratchpad_panes.remove(&key);
            self.scratchpad_focus_times.remove(&key);
        }

        // Clean up session-scoped scratchpads whose panes no longer exist
        let closed_session_scratchpads: Vec<String> = self
            .session_scratchpad_panes
            .iter()
            .filter(|(_, &pane_id)| !existing_pane_ids.contains(&pane_id))
            .map(|(name, _)| name.clone())
            .collect();

        for name in closed_session_scratchpads {
            self.session_scratchpad_panes.remove(&name);
        }

        // Clean up stale pending registrations for tabs that no longer exist
        let existing_stable_ids: HashSet<StableTabId> =
            self.stable_tab_to_position.keys().copied().collect();
        self.tab_pending_registrations
            .retain(|(_, stable_id)| existing_stable_ids.contains(stable_id));

        // Close scratchpad panes whose stable tab IDs are orphaned (tab was closed)
        // These scratchpads are no longer associated with any tab
        let orphaned_scratchpads: Vec<((String, StableTabId), u32)> = self
            .tab_scratchpad_panes
            .iter()
            .filter(|((_, stable_id), _)| !existing_stable_ids.contains(stable_id))
            .map(|(key, &pane_id)| (key.clone(), pane_id))
            .collect();

        for ((name, stable_id), pane_id) in orphaned_scratchpads {
            close_terminal_pane(pane_id);
            self.tab_scratchpad_panes.remove(&(name.clone(), stable_id));
            self.scratchpad_focus_times.remove(&(name, stable_id));
        }
    }

    /// Close scratchpad panes that have exited (ghost panes).
    /// This handles the case where the command in a scratchpad exits but
    /// Zellij keeps the pane open with "exited" status.
    fn close_exited_scratchpads(&mut self) {
        // Find all scratchpad panes that have exited
        let exited_pane_ids: HashSet<u32> = self
            .pane_manifest
            .values()
            .flatten()
            .filter(|p| p.exited || p.is_held)
            .map(|p| p.id)
            .collect();

        // Close exited tab-scoped scratchpads
        let exited_tab_keys: Vec<((String, StableTabId), u32)> = self
            .tab_scratchpad_panes
            .iter()
            .filter(|(_, &pane_id)| exited_pane_ids.contains(&pane_id))
            .map(|(key, &pane_id)| (key.clone(), pane_id))
            .collect();

        for (key, pane_id) in exited_tab_keys {
            close_terminal_pane(pane_id);
            self.tab_scratchpad_panes.remove(&key);
            self.scratchpad_focus_times.remove(&key);
        }

        // Close exited session-scoped scratchpads
        let exited_session_scratchpads: Vec<(String, u32)> = self
            .session_scratchpad_panes
            .iter()
            .filter(|(_, &pane_id)| exited_pane_ids.contains(&pane_id))
            .map(|(name, &pane_id)| (name.clone(), pane_id))
            .collect();

        for (name, pane_id) in exited_session_scratchpads {
            close_terminal_pane(pane_id);
            self.session_scratchpad_panes.remove(&name);
        }
    }

    fn get_pane_tab(&self, pane_id: u32) -> Option<usize> {
        // Priority order for finding the "real" pane location:
        // 1. Floating + not suppressed (visible floating - ideal state)
        // 2. Floating + suppressed (hidden floating - shouldn't happen but handle it)
        // 3. Suppressed (hidden pane - the real one we want to show/move)
        // 4. Any entry (ghost - non-floating, non-suppressed artifact)
        //
        // This matters because:
        // - Session scratchpads are meant to be floating
        // - Ghost entries (is_floating=false, is_suppressed=false) appear on other tabs
        // - When hidden, real pane is is_floating=false, is_suppressed=true
        // - HashMap iteration order is non-deterministic

        // Priority 1: visible floating
        self.pane_manifest
            .iter()
            .find(|(_, panes)| {
                panes
                    .iter()
                    .any(|p| p.id == pane_id && p.is_floating && !p.is_suppressed)
            })
            .or_else(|| {
                // Priority 2: suppressed floating
                self.pane_manifest.iter().find(|(_, panes)| {
                    panes
                        .iter()
                        .any(|p| p.id == pane_id && p.is_floating && p.is_suppressed)
                })
            })
            .or_else(|| {
                // Priority 3: suppressed (hidden pane, even if not floating)
                self.pane_manifest
                    .iter()
                    .find(|(_, panes)| panes.iter().any(|p| p.id == pane_id && p.is_suppressed))
            })
            .or_else(|| {
                // Priority 4: any entry (likely a ghost)
                self.pane_manifest
                    .iter()
                    .find(|(_, panes)| panes.iter().any(|p| p.id == pane_id))
            })
            .map(|(tab, _)| *tab)
    }

    fn get_scratchpad_scope(&self, name: &str) -> Option<ScratchpadScope> {
        self.scratchpad_configs.get(name).map(|c| c.scope)
    }

    /// Find a tab-scoped scratchpad's pane_id by name on the current tab (using stable ID)
    fn get_tab_scratchpad_pane(&self, name: &str) -> Option<u32> {
        let stable_tab_id = self.get_current_stable_tab_id()?;
        let key = (name.to_string(), stable_tab_id);
        self.tab_scratchpad_panes.get(&key).copied()
    }

    fn get_hidden_floating_pane_ids(&self) -> HashSet<u32> {
        self.pane_manifest
            .values()
            .flatten()
            .filter(|p| p.is_floating && p.is_suppressed)
            .map(|p| p.id)
            .collect()
    }

    /// Get the pane_id for a scratchpad by name (checking both tab and session scope)
    fn get_scratchpad_pane_id(&self, name: &str) -> Option<u32> {
        match self.get_scratchpad_scope(name) {
            Some(ScratchpadScope::Session) => self.session_scratchpad_panes.get(name).copied(),
            Some(ScratchpadScope::Tab) => self.get_tab_scratchpad_pane(name),
            None => None,
        }
    }

    fn is_scratchpad_visible(&self, name: &str) -> bool {
        // If floating panes are not visible (tiled layer is focused), no scratchpad is visible
        if !self.are_floating_panes_visible {
            return false;
        }

        let Some(pane_id) = self.get_scratchpad_pane_id(name) else {
            return false;
        };

        // Check that the pane is visible (not suppressed, not exited) on current tab
        // We specifically check for floating panes since scratchpads are floating
        self.pane_manifest
            .get(&self.current_tab_position)
            .into_iter()
            .flatten()
            .any(|p| {
                p.id == pane_id && p.is_floating && !p.is_suppressed && !p.exited && !p.is_held
            })
    }

    fn is_scratchpad_focused(&self, name: &str) -> bool {
        let Some(pane_id) = self.get_scratchpad_pane_id(name) else {
            return false;
        };

        // If floating panes are not visible, the floating layer doesn't have focus
        if !self.are_floating_panes_visible {
            return false;
        }

        // Check if we just showed this pane (haven't received PaneUpdate yet)
        if self.just_shown_scratchpad == Some(pane_id) {
            return true;
        }

        // Get our scratchpad pane info - must be floating (scratchpads are floating panes)
        self.pane_manifest
            .values()
            .flatten()
            .find(|p| p.id == pane_id && p.is_floating)
            .map(|p| p.is_focused)
            .unwrap_or(false)
    }

    fn get_focused_scratchpad(&self) -> Option<String> {
        // If floating layer isn't visible, no scratchpad can be focused
        if !self.are_floating_panes_visible {
            return None;
        }

        // Find the focused floating pane (scratchpads are floating)
        let focused_pane_id = self
            .pane_manifest
            .values()
            .flatten()
            .find(|p| p.is_floating && p.is_focused)?
            .id;

        // Check session-scoped scratchpads
        if let Some(name) = self
            .session_scratchpad_panes
            .iter()
            .find(|(_, &pane_id)| pane_id == focused_pane_id)
            .map(|(name, _)| name.clone())
        {
            return Some(name);
        }

        // Check tab-scoped scratchpads
        self.tab_scratchpad_panes
            .iter()
            .find(|(_, &pane_id)| pane_id == focused_pane_id)
            .map(|((name, _), _)| name.clone())
    }

    /// Update focus tracking for the currently focused scratchpad (if any).
    /// This should be called on every PaneUpdate to track focus changes
    /// that happen outside of our plugin's actions (e.g., user clicking on a scratchpad).
    fn update_focused_scratchpad_tracking(&mut self) {
        // If floating layer isn't visible, no scratchpad can be focused
        if !self.are_floating_panes_visible {
            return;
        }

        // Find the focused floating pane
        let focused_pane = self
            .pane_manifest
            .values()
            .flatten()
            .find(|p| p.is_floating && p.is_focused);

        let Some(focused) = focused_pane else {
            return;
        };

        let focused_pane_id = focused.id;

        // Check if this is a tab-scoped scratchpad and update its focus time
        if let Some((key, _)) = self
            .tab_scratchpad_panes
            .iter()
            .find(|(_, &pane_id)| pane_id == focused_pane_id)
        {
            let key = key.clone();
            self.focus_counter += 1;
            self.scratchpad_focus_times.insert(key, self.focus_counter);
        }
        // Note: session-scoped scratchpads don't track per-tab focus times
    }

    fn build_shim_command(&self, name: &str, config: &ScratchpadConfig) -> CommandToRun {
        // Both scopes use the same format now - we derive tab from pane_manifest
        let scope_str = match config.scope {
            ScratchpadScope::Session => "session",
            ScratchpadScope::Tab => "tab",
        };
        let register_msg = format!(
            r#"zellij pipe "zellij-tools::scratchpad::register::{}::{}::$ZELLIJ_PANE_ID""#,
            scope_str, name
        );

        let mut args = vec![
            "-c".to_string(),
            format!(r#"{} && exec "$@""#, register_msg),
            "_".to_string(), // $0 placeholder
        ];
        args.extend(config.command.clone());

        CommandToRun::new_with_args("sh", args)
    }

    fn handle_scratchpad_show(&mut self, name: &str) {
        // Check if scratchpad is configured
        let config = match self.scratchpad_configs.get(name) {
            Some(c) => c.clone(),
            None => return, // Silent no-op for unknown scratchpad
        };

        match config.scope {
            ScratchpadScope::Tab => self.handle_tab_scratchpad_show(name, &config),
            ScratchpadScope::Session => self.handle_session_scratchpad_show(name, &config),
        }
    }

    fn handle_tab_scratchpad_show(&mut self, name: &str, config: &ScratchpadConfig) {
        let Some(stable_tab_id) = self.get_current_stable_tab_id() else {
            return;
        };

        // Check if there's already a scratchpad with this name on the current tab
        let existing_pane_id = self.get_tab_scratchpad_pane(name);

        // If not yet registered on this tab, spawn the pane
        if existing_pane_id.is_none() {
            let pending_key = (name.to_string(), stable_tab_id);
            if self.tab_pending_registrations.contains(&pending_key) {
                return; // Already spawning on this tab
            }

            self.tab_pending_registrations.insert(pending_key);
            let cmd = self.build_shim_command(name, config);
            open_command_pane_floating(cmd, None, BTreeMap::new());
            return;
        }

        // Already registered - show the pane
        self.show_scratchpad_pane(name, existing_pane_id.unwrap(), stable_tab_id);
    }

    fn handle_session_scratchpad_show(&mut self, name: &str, config: &ScratchpadConfig) {
        // If not yet registered, spawn the pane
        if !self.session_scratchpad_panes.contains_key(name) {
            if self.session_pending_registrations.contains(name) {
                return; // Already spawning
            }

            self.session_pending_registrations.insert(name.to_string());
            let cmd = self.build_shim_command(name, config);
            open_command_pane_floating(cmd, None, BTreeMap::new());
            return;
        }

        // Already registered - check if on current tab
        let pane_id = *self.session_scratchpad_panes.get(name).unwrap();
        let pane_tab = self.get_pane_tab(pane_id);

        // If on different tab, we need to show it first (to make it floating),
        // then move it to the current tab.
        //
        // Why: Zellij's suppress_pane() removes panes from floating_panes into
        // suppressed_panes. When break_panes_to_tab_with_index() moves a pane,
        // it checks pane_id_is_floating() which only looks in floating_panes.
        // A suppressed pane returns false, so it gets added as tiled on the target tab.
        //
        // Solution: show_pane_with_id(_, true) unsuppresses as floating.
        // Then break_panes_to_tab_with_index() sees it as floating and preserves that.
        if pane_tab != Some(self.current_tab_position) {
            // Check if target tab has a ghost entry for this pane
            // Ghosts are is_floating=false, is_suppressed=false entries
            let target_has_ghost = self
                .pane_manifest
                .get(&self.current_tab_position)
                .map(|panes| {
                    panes
                        .iter()
                        .any(|p| p.id == pane_id && !p.is_floating && !p.is_suppressed)
                })
                .unwrap_or(false);

            if target_has_ghost {
                // Target tab has a ghost - Zellij's move will fail
                // Fallback: just show the pane on its current tab (user gets switched there)
                show_pane_with_id(PaneId::Terminal(pane_id), true);
                return;
            }

            // Show the pane (unsuppresses as floating, but switches us to its tab)
            show_pane_with_id(PaneId::Terminal(pane_id), true);
            // Move the pane to current tab AND switch focus back to current tab
            break_panes_to_tab_with_index(
                &[PaneId::Terminal(pane_id)],
                self.current_tab_position,
                false,
            );
            // Toggle to floating (in case it became tiled during move)
            toggle_pane_embed_or_eject_for_pane_id(PaneId::Terminal(pane_id));
            // Show the pane
            self.show_scratchpad_pane_session(name, pane_id);
            return;
        }

        self.show_scratchpad_pane_session(name, pane_id);
    }

    fn show_scratchpad_pane(&mut self, name: &str, pane_id: u32, stable_tab_id: StableTabId) {
        // Capture which panes are currently hidden BEFORE showing
        let hidden_before = self.get_hidden_floating_pane_ids();

        // Show the scratchpad pane (this will show ALL floating panes as a side effect)
        show_pane_with_id(PaneId::Terminal(pane_id), true);

        // Track that we just showed this pane (for focus detection before PaneUpdate arrives)
        self.just_shown_scratchpad = Some(pane_id);

        // Re-hide all panes that were hidden before (except our scratchpad)
        for hidden_pane_id in hidden_before {
            if hidden_pane_id != pane_id {
                hide_pane_with_id(PaneId::Terminal(hidden_pane_id));
            }
        }

        // Update focus tracking (monotonic counter for recency)
        self.focus_counter += 1;
        let key = (name.to_string(), stable_tab_id);
        self.scratchpad_focus_times.insert(key, self.focus_counter);
    }

    fn show_scratchpad_pane_session(&mut self, _name: &str, pane_id: u32) {
        // Capture which panes are currently hidden BEFORE showing
        let hidden_before = self.get_hidden_floating_pane_ids();

        // Show the scratchpad pane (this will show ALL floating panes as a side effect)
        show_pane_with_id(PaneId::Terminal(pane_id), true);

        // Track that we just showed this pane (for focus detection before PaneUpdate arrives)
        self.just_shown_scratchpad = Some(pane_id);

        // Re-hide all panes that were hidden before (except our scratchpad)
        for hidden_pane_id in hidden_before {
            if hidden_pane_id != pane_id {
                hide_pane_with_id(PaneId::Terminal(hidden_pane_id));
            }
        }
    }

    fn handle_scratchpad_hide(&mut self, name: &str) {
        let pane_id = match self.get_scratchpad_scope(name) {
            Some(ScratchpadScope::Session) => self.session_scratchpad_panes.get(name).copied(),
            Some(ScratchpadScope::Tab) => self.get_tab_scratchpad_pane(name),
            None => return,
        };

        if let Some(pane_id) = pane_id {
            hide_pane_with_id(PaneId::Terminal(pane_id));
        }
    }

    fn handle_scratchpad_close(&mut self, name: &str) {
        match self.get_scratchpad_scope(name) {
            Some(ScratchpadScope::Session) => {
                if let Some(pane_id) = self.session_scratchpad_panes.remove(name) {
                    close_terminal_pane(pane_id);
                }
            }
            Some(ScratchpadScope::Tab) => {
                if let Some(stable_tab_id) = self.get_current_stable_tab_id() {
                    let key = (name.to_string(), stable_tab_id);
                    if let Some(pane_id) = self.tab_scratchpad_panes.remove(&key) {
                        close_terminal_pane(pane_id);
                        self.scratchpad_focus_times.remove(&key);
                    }
                }
            }
            None => {} // Unknown scratchpad - no-op
        }
    }

    fn handle_scratchpad_register_tab(&mut self, name: &str, pane_id: u32) {
        let pane_tab_position = self.get_pane_tab(pane_id);

        // Find which stable tab ID this pane was intended for by looking at pending registrations
        let intended_stable_id = self
            .tab_pending_registrations
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, stable_id)| *stable_id);

        // Get the current stable tab ID as fallback
        let target_stable_id = intended_stable_id.or_else(|| self.get_current_stable_tab_id());
        let Some(stable_tab_id) = target_stable_id else {
            return;
        };

        // Clear pending registration
        self.tab_pending_registrations
            .remove(&(name.to_string(), stable_tab_id));

        // Get the target tab position for this stable ID
        let target_tab_position = self.stable_tab_to_position.get(&stable_tab_id).copied();

        // If the pane is on the wrong tab, move it to the intended tab
        if let (Some(actual_tab), Some(target_tab)) = (pane_tab_position, target_tab_position) {
            if actual_tab != target_tab {
                break_panes_to_tab_with_index(&[PaneId::Terminal(pane_id)], target_tab, false);
            }
        }

        let key = (name.to_string(), stable_tab_id);
        self.tab_scratchpad_panes.insert(key.clone(), pane_id);

        // Track that we just showed this pane (newly spawned panes are focused)
        self.just_shown_scratchpad = Some(pane_id);

        // Update focus tracking
        self.focus_counter += 1;
        self.scratchpad_focus_times.insert(key, self.focus_counter);

        // Re-hide any floating panes that should be hidden
        let hidden_panes = self.get_hidden_floating_pane_ids();
        for hidden_pane_id in hidden_panes {
            if hidden_pane_id != pane_id {
                hide_pane_with_id(PaneId::Terminal(hidden_pane_id));
            }
        }
    }

    fn handle_scratchpad_register_session(&mut self, name: &str, pane_id: u32) {
        self.session_pending_registrations.remove(name);
        self.session_scratchpad_panes
            .insert(name.to_string(), pane_id);

        // Track that we just showed this pane (newly spawned panes are focused)
        self.just_shown_scratchpad = Some(pane_id);

        // Re-hide any floating panes that should be hidden
        let hidden_panes = self.get_hidden_floating_pane_ids();
        for hidden_pane_id in hidden_panes {
            if hidden_pane_id != pane_id {
                hide_pane_with_id(PaneId::Terminal(hidden_pane_id));
            }
        }
    }

    /// Get the most recently focused scratchpad on the current tab
    fn get_last_focused_scratchpad_on_current_tab(&self) -> Option<String> {
        let stable_tab_id = self.get_current_stable_tab_id()?;

        // Find the scratchpad with the highest focus time on current stable tab
        self.scratchpad_focus_times
            .iter()
            .filter(|((_, sid), _)| *sid == stable_tab_id)
            .max_by_key(|(_, &focus_time)| focus_time)
            .map(|((name, _), _)| name.clone())
    }

    fn handle_scratchpad_toggle(&mut self, name: Option<String>) {
        let target_name = match name {
            // Explicit name provided
            Some(n) => n,
            // No name - check if a scratchpad is focused
            None => {
                if let Some(focused) = self.get_focused_scratchpad() {
                    focused
                } else {
                    // No focused scratchpad - use last from current tab's focus history
                    match self.get_last_focused_scratchpad_on_current_tab() {
                        Some(last) => last,
                        None => return, // No history - no-op
                    }
                }
            }
        };

        // Check if configured
        if !self.scratchpad_configs.contains_key(&target_name) {
            return; // Silent no-op for unknown scratchpad
        }

        // Toggle based on current visibility and focus state
        // - If visible AND focused → hide it
        // - If visible but NOT focused → focus it (show without hiding)
        // - If not visible → show it
        let visible = self.is_scratchpad_visible(&target_name);
        let focused = self.is_scratchpad_focused(&target_name);

        if visible && focused {
            self.handle_scratchpad_hide(&target_name);
        } else {
            // Either not visible (show it) or visible but not focused (focus it)
            self.handle_scratchpad_show(&target_name);
        }
    }

    fn handle_scratchpad_action(&mut self, action: ScratchpadAction) {
        match action {
            ScratchpadAction::Toggle { name } => self.handle_scratchpad_toggle(name),
            ScratchpadAction::Show { name } => self.handle_scratchpad_show(&name),
            ScratchpadAction::Hide { name } => self.handle_scratchpad_hide(&name),
            ScratchpadAction::Close { name } => self.handle_scratchpad_close(&name),
            ScratchpadAction::RegisterTab { name, pane_id } => {
                self.handle_scratchpad_register_tab(&name, pane_id)
            }
            ScratchpadAction::RegisterSession { name, pane_id } => {
                self.handle_scratchpad_register_session(&name, pane_id)
            }
        }
    }

    fn handle_event(&mut self, event: &str, args: Vec<String>) -> Result<(), ParseError> {
        match event {
            "focus-pane" => {
                if args.len() != 1 {
                    return Err(ParseError::InvalidArgs(format!(
                        "focus-pane requires 1 argument, got {}",
                        args.len()
                    )));
                }

                let pane_id = args[0]
                    .parse::<PaneId>()
                    .map_err(|e| ParseError::InvalidArgs(format!("Invalid pane ID: {}", e)))?;

                show_pane_with_id(pane_id, true);
                Ok(())
            }
            "scratchpad" => {
                let action = self.parse_scratchpad_action(&args)?;
                self.handle_scratchpad_action(action);
                Ok(())
            }
            _ => Err(ParseError::UnknownEvent(event.to_string())),
        }
    }
}

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        eprintln!("load: {:?}", configuration);

        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
        ]);

        subscribe(&[EventType::PaneUpdate, EventType::TabUpdate]);

        // Parse scratchpad configuration from JSON
        if let Some(scratchpads_json) = configuration.get("scratchpads") {
            match serde_json::from_str::<HashMap<String, ScratchpadConfig>>(scratchpads_json) {
                Ok(configs) => {
                    // Validate all scratchpad names
                    for name in configs.keys() {
                        if !is_valid_scratchpad_name(name) {
                            eprintln!("Warning: Invalid scratchpad name '{}', skipping", name);
                        }
                    }
                    self.scratchpad_configs = configs
                        .into_iter()
                        .filter(|(name, _)| is_valid_scratchpad_name(name))
                        .collect();
                }
                Err(e) => {
                    eprintln!("Failed to parse scratchpads config: {}", e);
                }
            }
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::PaneUpdate(pane_manifest) => {
                eprintln!("PaneUpdate: {:?}", pane_manifest);
                self.pane_manifest = pane_manifest.panes;

                // Clear the "just shown" tracking since we now have fresh state
                self.just_shown_scratchpad = None;
                // Update stable tab mapping FIRST, before other operations
                self.update_stable_tab_mapping();
                // Track focus changes that happen outside of our plugin's actions
                self.update_focused_scratchpad_tracking();
                self.close_exited_scratchpads();
                self.cleanup_closed_scratchpads();
            }
            Event::TabUpdate(tab_infos) => {
                eprintln!("TabUpdate: {:?}", tab_infos);
                // Find the active tab and update state
                if let Some(active_tab) = tab_infos.iter().find(|t| t.active) {
                    self.current_tab_position = active_tab.position;
                    self.are_floating_panes_visible = active_tab.are_floating_panes_visible;
                } else {
                    eprintln!("Warning: No active tab found");
                }
            }
            _ => (),
        };
        false
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        let payload = match pipe_message.payload {
            Some(p) => p,
            None => return false,
        };

        match self.parse_message(&payload) {
            Ok((event, args)) => match self.handle_event(&event, args) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Error handling event: {}", e);
                    false
                }
            },
            Err(e) => {
                eprintln!("Error parsing message: {}", e);
                false
            }
        }
    }
}
