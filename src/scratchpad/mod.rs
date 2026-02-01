mod config;

pub use config::{
    is_valid_scratchpad_name, parse_scratchpad_action, ScratchpadAction, ScratchpadConfig,
};

use std::collections::{HashMap, HashSet};
use zellij_tile::prelude::{CommandToRun, PaneInfo};

use crate::stable_tabs::StableTabId;

/// Commands returned by ScratchpadManager for the caller to execute
#[derive(Debug)]
pub enum ScratchpadCommand {
    /// Open a new floating pane with the given command
    OpenFloating { command: CommandToRun },
    /// Show a pane (make visible and focus)
    ShowPane { pane_id: u32 },
    /// Hide a pane (suppress)
    HidePane { pane_id: u32 },
    /// Close a pane
    ClosePane { pane_id: u32 },
    /// Move a pane to a different tab
    MovePaneToTab { pane_id: u32, tab_position: usize },
}

/// Context needed by ScratchpadManager to make decisions
pub struct ScratchpadContext<'a> {
    pub pane_manifest: &'a HashMap<usize, Vec<PaneInfo>>,
    pub current_tab_position: usize,
    pub current_stable_tab_id: Option<StableTabId>,
    pub are_floating_panes_visible: bool,
    pub stable_tab_to_position: &'a HashMap<StableTabId, usize>,
}

/// Manages scratchpad state and returns commands to execute
pub struct ScratchpadManager {
    configs: HashMap<String, ScratchpadConfig>,
    /// (name, stable_tab_id) -> pane_id
    panes: HashMap<(String, StableTabId), u32>,
    /// Pending registrations: (name, stable_tab_id)
    pending: HashSet<(String, StableTabId)>,
    /// Monotonic counter for focus tracking
    focus_counter: u64,
    /// (name, stable_tab_id) -> last focus timestamp
    focus_times: HashMap<(String, StableTabId), u64>,
    /// Track scratchpad we just showed (for focus detection before PaneUpdate)
    just_shown: Option<u32>,
}

impl ScratchpadManager {
    pub fn new(configs: HashMap<String, ScratchpadConfig>) -> Self {
        Self {
            configs,
            panes: HashMap::new(),
            pending: HashSet::new(),
            focus_counter: 0,
            focus_times: HashMap::new(),
            just_shown: None,
        }
    }

    /// Clear the "just shown" tracking (call on PaneUpdate)
    pub fn clear_just_shown(&mut self) {
        self.just_shown = None;
    }

    /// Handle a scratchpad action, returns commands to execute
    pub fn handle_action(
        &mut self,
        action: ScratchpadAction,
        ctx: &ScratchpadContext,
    ) -> Vec<ScratchpadCommand> {
        match action {
            ScratchpadAction::Toggle { name } => self.handle_toggle(name, ctx),
            ScratchpadAction::Show { name } => self.handle_show(&name, ctx),
            ScratchpadAction::Hide { name } => self.handle_hide(&name, ctx),
            ScratchpadAction::Close { name } => self.handle_close(&name, ctx),
            ScratchpadAction::Register { name, pane_id } => {
                self.handle_register(&name, pane_id, ctx)
            }
        }
    }

    /// Called on PaneUpdate to sync state and clean up
    pub fn on_pane_update(
        &mut self,
        ctx: &ScratchpadContext,
        orphaned_tabs: &HashSet<StableTabId>,
    ) -> Vec<ScratchpadCommand> {
        let mut commands = Vec::new();

        // Track focus changes that happen outside of our actions
        self.update_focus_tracking(ctx);

        // Close exited scratchpads
        commands.extend(self.close_exited_scratchpads(ctx));

        // Clean up scratchpads for closed panes
        self.cleanup_closed_panes(ctx);

        // Clean up pending registrations for orphaned tabs
        self.pending
            .retain(|(_, stable_id)| !orphaned_tabs.contains(stable_id));

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
                // Check if a scratchpad is focused
                if let Some(focused) = self.get_focused_scratchpad(ctx) {
                    focused
                } else {
                    // Use last from current tab's focus history
                    match self.get_last_focused_on_current_tab(ctx) {
                        Some(last) => last,
                        None => return Vec::new(),
                    }
                }
            }
        };

        // Check if configured
        if !self.configs.contains_key(&target_name) {
            return Vec::new();
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

        let Some(stable_tab_id) = ctx.current_stable_tab_id else {
            return Vec::new();
        };

        let existing_pane_id = self.get_pane(name, ctx);

        if existing_pane_id.is_none() {
            let pending_key = (name.to_string(), stable_tab_id);
            if self.pending.contains(&pending_key) {
                return Vec::new();
            }

            self.pending.insert(pending_key);
            let cmd = self.build_shim_command(name, &config);
            return vec![ScratchpadCommand::OpenFloating { command: cmd }];
        }

        let pane_id = existing_pane_id.unwrap();
        self.show_pane(name, pane_id, stable_tab_id, ctx)
    }

    fn handle_hide(&mut self, name: &str, ctx: &ScratchpadContext) -> Vec<ScratchpadCommand> {
        if let Some(pane_id) = self.get_pane(name, ctx) {
            vec![ScratchpadCommand::HidePane { pane_id }]
        } else {
            Vec::new()
        }
    }

    fn handle_close(&mut self, name: &str, ctx: &ScratchpadContext) -> Vec<ScratchpadCommand> {
        let Some(stable_tab_id) = ctx.current_stable_tab_id else {
            return Vec::new();
        };

        let key = (name.to_string(), stable_tab_id);
        if let Some(pane_id) = self.panes.remove(&key) {
            self.focus_times.remove(&key);
            vec![ScratchpadCommand::ClosePane { pane_id }]
        } else {
            Vec::new()
        }
    }

    fn handle_register(
        &mut self,
        name: &str,
        pane_id: u32,
        ctx: &ScratchpadContext,
    ) -> Vec<ScratchpadCommand> {
        let mut commands = Vec::new();

        let pane_tab_position = self.get_pane_tab(pane_id, ctx);

        // Find which stable tab ID this pane was intended for
        let intended_stable_id = self
            .pending
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, stable_id)| *stable_id);

        let target_stable_id = intended_stable_id.or(ctx.current_stable_tab_id);
        let Some(stable_tab_id) = target_stable_id else {
            return commands;
        };

        self.pending.remove(&(name.to_string(), stable_tab_id));

        let target_tab_position = ctx.stable_tab_to_position.get(&stable_tab_id).copied();

        // Move pane if on wrong tab
        if let (Some(actual_tab), Some(target_tab)) = (pane_tab_position, target_tab_position) {
            if actual_tab != target_tab {
                commands.push(ScratchpadCommand::MovePaneToTab {
                    pane_id,
                    tab_position: target_tab,
                });
            }
        }

        let key = (name.to_string(), stable_tab_id);
        self.panes.insert(key.clone(), pane_id);
        self.just_shown = Some(pane_id);
        self.focus_counter += 1;
        self.focus_times.insert(key, self.focus_counter);

        // Re-hide any floating panes that should be hidden
        for hidden_pane_id in self.get_hidden_floating_pane_ids(ctx) {
            if hidden_pane_id != pane_id {
                commands.push(ScratchpadCommand::HidePane {
                    pane_id: hidden_pane_id,
                });
            }
        }

        commands
    }

    fn show_pane(
        &mut self,
        name: &str,
        pane_id: u32,
        stable_tab_id: StableTabId,
        ctx: &ScratchpadContext,
    ) -> Vec<ScratchpadCommand> {
        let mut commands = Vec::new();
        let hidden_before = self.get_hidden_floating_pane_ids(ctx);

        commands.push(ScratchpadCommand::ShowPane { pane_id });
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
        let key = (name.to_string(), stable_tab_id);
        self.focus_times.insert(key, self.focus_counter);

        commands
    }

    fn build_shim_command(&self, name: &str, config: &ScratchpadConfig) -> CommandToRun {
        let register_msg = format!(
            r#"zellij pipe "zellij-tools::scratchpad::register::{}::$ZELLIJ_PANE_ID""#,
            name
        );

        let mut args = vec![
            "-c".to_string(),
            format!(r#"{} && exec "$@""#, register_msg),
            "_".to_string(),
        ];
        args.extend(config.command.clone());

        CommandToRun::new_with_args("sh", args)
    }

    fn get_pane(&self, name: &str, ctx: &ScratchpadContext) -> Option<u32> {
        let stable_tab_id = ctx.current_stable_tab_id?;
        let key = (name.to_string(), stable_tab_id);
        self.panes.get(&key).copied()
    }

    fn get_pane_tab(&self, pane_id: u32, ctx: &ScratchpadContext) -> Option<usize> {
        ctx.pane_manifest
            .iter()
            .find(|(_, panes)| panes.iter().any(|p| p.id == pane_id))
            .map(|(tab, _)| *tab)
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
        if !ctx.are_floating_panes_visible {
            return false;
        }

        let Some(pane_id) = self.get_pane(name, ctx) else {
            return false;
        };

        ctx.pane_manifest
            .get(&ctx.current_tab_position)
            .into_iter()
            .flatten()
            .any(|p| {
                p.id == pane_id && p.is_floating && !p.is_suppressed && !p.exited && !p.is_held
            })
    }

    fn is_focused(&self, name: &str, ctx: &ScratchpadContext) -> bool {
        let Some(pane_id) = self.get_pane(name, ctx) else {
            return false;
        };

        if !ctx.are_floating_panes_visible {
            return false;
        }

        if self.just_shown == Some(pane_id) {
            return true;
        }

        ctx.pane_manifest
            .values()
            .flatten()
            .find(|p| p.id == pane_id && p.is_floating)
            .map(|p| p.is_focused)
            .unwrap_or(false)
    }

    fn get_focused_scratchpad(&self, ctx: &ScratchpadContext) -> Option<String> {
        if !ctx.are_floating_panes_visible {
            return None;
        }

        let focused_pane_id = ctx
            .pane_manifest
            .values()
            .flatten()
            .find(|p| p.is_floating && p.is_focused)?
            .id;

        self.panes
            .iter()
            .find(|(_, &pane_id)| pane_id == focused_pane_id)
            .map(|((name, _), _)| name.clone())
    }

    fn get_last_focused_on_current_tab(&self, ctx: &ScratchpadContext) -> Option<String> {
        let stable_tab_id = ctx.current_stable_tab_id?;

        self.focus_times
            .iter()
            .filter(|((_, sid), _)| *sid == stable_tab_id)
            .max_by_key(|(_, &focus_time)| focus_time)
            .map(|((name, _), _)| name.clone())
    }

    fn update_focus_tracking(&mut self, ctx: &ScratchpadContext) {
        if !ctx.are_floating_panes_visible {
            return;
        }

        let focused_pane = ctx
            .pane_manifest
            .values()
            .flatten()
            .find(|p| p.is_floating && p.is_focused);

        let Some(focused) = focused_pane else {
            return;
        };

        if let Some((key, _)) = self
            .panes
            .iter()
            .find(|(_, &pane_id)| pane_id == focused.id)
        {
            let key = key.clone();
            self.focus_counter += 1;
            self.focus_times.insert(key, self.focus_counter);
        }
    }

    fn close_exited_scratchpads(&mut self, ctx: &ScratchpadContext) -> Vec<ScratchpadCommand> {
        let exited_pane_ids: HashSet<u32> = ctx
            .pane_manifest
            .values()
            .flatten()
            .filter(|p| p.exited || p.is_held)
            .map(|p| p.id)
            .collect();

        let exited_keys: Vec<((String, StableTabId), u32)> = self
            .panes
            .iter()
            .filter(|(_, &pane_id)| exited_pane_ids.contains(&pane_id))
            .map(|(key, &pane_id)| (key.clone(), pane_id))
            .collect();

        let mut commands = Vec::new();
        for (key, pane_id) in exited_keys {
            commands.push(ScratchpadCommand::ClosePane { pane_id });
            self.panes.remove(&key);
            self.focus_times.remove(&key);
        }
        commands
    }

    fn cleanup_closed_panes(&mut self, ctx: &ScratchpadContext) {
        let existing_pane_ids: HashSet<u32> =
            ctx.pane_manifest.values().flatten().map(|p| p.id).collect();

        let closed_keys: Vec<(String, StableTabId)> = self
            .panes
            .iter()
            .filter(|(_, &pane_id)| !existing_pane_ids.contains(&pane_id))
            .map(|(key, _)| key.clone())
            .collect();

        for key in closed_keys {
            self.panes.remove(&key);
            self.focus_times.remove(&key);
        }
    }

    fn close_orphaned_scratchpads(
        &mut self,
        orphaned_tabs: &HashSet<StableTabId>,
    ) -> Vec<ScratchpadCommand> {
        let orphaned: Vec<((String, StableTabId), u32)> = self
            .panes
            .iter()
            .filter(|((_, stable_id), _)| orphaned_tabs.contains(stable_id))
            .map(|(key, &pane_id)| (key.clone(), pane_id))
            .collect();

        let mut commands = Vec::new();
        for ((name, stable_id), pane_id) in orphaned {
            commands.push(ScratchpadCommand::ClosePane { pane_id });
            self.panes.remove(&(name.clone(), stable_id));
            self.focus_times.remove(&(name, stable_id));
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
        stable_tab_id: Option<StableTabId>,
        floating_visible: bool,
        stable_tab_to_position: &'a HashMap<StableTabId, usize>,
    ) -> ScratchpadContext<'a> {
        ScratchpadContext {
            pane_manifest,
            current_tab_position: current_tab,
            current_stable_tab_id: stable_tab_id,
            are_floating_panes_visible: floating_visible,
            stable_tab_to_position,
        }
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

        manager.handle_action(
            ScratchpadAction::Register {
                name: "term".to_string(),
                pane_id: 42,
            },
            &ctx,
        );

        // Now show it
        let commands = manager.handle_action(
            ScratchpadAction::Show {
                name: "term".to_string(),
            },
            &ctx,
        );

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::ShowPane { pane_id: 42 })));
    }

    #[test]
    fn hide_scratchpad_hides_pane() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.handle_action(
            ScratchpadAction::Register {
                name: "term".to_string(),
                pane_id: 42,
            },
            &ctx,
        );

        let commands = manager.handle_action(
            ScratchpadAction::Hide {
                name: "term".to_string(),
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

        manager.handle_action(
            ScratchpadAction::Register {
                name: "term".to_string(),
                pane_id: 42,
            },
            &ctx,
        );

        let commands = manager.handle_action(
            ScratchpadAction::Close {
                name: "term".to_string(),
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

        manager.handle_action(
            ScratchpadAction::Register {
                name: "term".to_string(),
                pane_id: 42,
            },
            &ctx,
        );
        manager.clear_just_shown(); // Simulate PaneUpdate

        let commands = manager.handle_action(
            ScratchpadAction::Toggle {
                name: Some("term".to_string()),
            },
            &ctx,
        );

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::ShowPane { pane_id: 42 })));
    }

    #[test]
    fn toggle_visible_focused_scratchpad_hides_it() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.handle_action(
            ScratchpadAction::Register {
                name: "term".to_string(),
                pane_id: 42,
            },
            &ctx,
        );

        let commands = manager.handle_action(
            ScratchpadAction::Toggle {
                name: Some("term".to_string()),
            },
            &ctx,
        );

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::HidePane { pane_id: 42 })));
    }

    #[test]
    fn on_pane_update_closes_exited_scratchpads() {
        let mut manager = ScratchpadManager::new(make_configs(&["term"]));

        // Register scratchpad
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_floating_pane(42, true)]);
        let mut positions = HashMap::new();
        positions.insert(0, 0);
        let ctx = make_context(&manifest, 0, Some(0), true, &positions);

        manager.handle_action(
            ScratchpadAction::Register {
                name: "term".to_string(),
                pane_id: 42,
            },
            &ctx,
        );

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

        manager.handle_action(
            ScratchpadAction::Register {
                name: "term".to_string(),
                pane_id: 42,
            },
            &ctx,
        );

        // Tab 0 is now orphaned
        let mut orphaned = HashSet::new();
        orphaned.insert(0);

        let commands = manager.on_pane_update(&ctx, &orphaned);

        assert!(commands
            .iter()
            .any(|c| matches!(c, ScratchpadCommand::ClosePane { pane_id: 42 })));
    }
}
