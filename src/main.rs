use std::collections::{BTreeMap, HashMap};

use zellij_tile::prelude::*;

use zellij_tools::message::{parse_message, ParseError};
use zellij_tools::scratchpad::{
    delete_state_file, load_state, parse_scratchpad_action, parse_scratchpads_kdl, save_state,
    ScratchpadCommand, ScratchpadContext, ScratchpadManager,
};
use zellij_tools::stable_tabs::StableTabTracker;

#[derive(Default)]
struct State {
    // Pane tracking (from PaneUpdate events)
    pane_manifest: HashMap<usize, Vec<PaneInfo>>,

    // Current tab position and floating pane visibility (from TabUpdate events)
    current_tab_position: usize,
    are_floating_panes_visible: bool,

    // Managers
    tab_tracker: StableTabTracker,
    scratchpad: Option<ScratchpadManager>,
}

register_plugin!(State);

impl State {
    fn build_scratchpad_context(&self) -> ScratchpadContext<'_> {
        ScratchpadContext {
            pane_manifest: &self.pane_manifest,
            current_tab_position: self.current_tab_position,
            current_stable_tab_id: self.tab_tracker.get_stable_id(self.current_tab_position),
            are_floating_panes_visible: self.are_floating_panes_visible,
            stable_tab_to_position: &self.tab_tracker.stable_tab_to_position,
        }
    }

    fn execute_scratchpad_commands(&self, commands: Vec<ScratchpadCommand>) {
        for cmd in commands {
            match cmd {
                ScratchpadCommand::OpenFloating { command } => {
                    open_command_pane_floating(command, None, BTreeMap::new());
                }
                ScratchpadCommand::ShowPane { pane_id } => {
                    show_pane_with_id(PaneId::Terminal(pane_id), true);
                }
                ScratchpadCommand::HidePane { pane_id } => {
                    hide_pane_with_id(PaneId::Terminal(pane_id));
                }
                ScratchpadCommand::ClosePane { pane_id } => {
                    close_terminal_pane(pane_id);
                }
                ScratchpadCommand::MovePaneToTab {
                    pane_id,
                    tab_position,
                } => {
                    break_panes_to_tab_with_index(
                        &[PaneId::Terminal(pane_id)],
                        tab_position,
                        false,
                    );
                }
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
                let action = parse_scratchpad_action(&args)?;
                if let Some(mut scratchpad) = self.scratchpad.take() {
                    let ctx = self.build_scratchpad_context();
                    let commands = scratchpad.handle_action(action, &ctx);
                    self.scratchpad = Some(scratchpad);
                    self.execute_scratchpad_commands(commands);
                }
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

        subscribe(&[
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::ConfigWasWrittenToDisk,
        ]);

        // Try to restore state from previous session
        let ids = get_plugin_ids();
        let restored_state = load_state(ids.zellij_pid);
        delete_state_file(ids.zellij_pid);

        // Parse scratchpad configuration from KDL
        if let Some(scratchpads_kdl) = configuration.get("scratchpads") {
            match parse_scratchpads_kdl(scratchpads_kdl) {
                Ok(configs) => {
                    let mut manager = ScratchpadManager::new(configs);

                    // Restore state from previous session if available
                    if let Some(state) = restored_state {
                        eprintln!("Restoring scratchpad state from previous session");
                        let commands = manager.restore_state(state);
                        // Execute commands to show orphaned panes
                        self.scratchpad = Some(manager);
                        self.execute_scratchpad_commands(commands);
                        return;
                    }

                    self.scratchpad = Some(manager);
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

                // Update stable tab mapping and get orphaned tabs
                let orphaned_tabs = self.tab_tracker.update(&self.pane_manifest);

                // Update scratchpad manager
                if let Some(mut scratchpad) = self.scratchpad.take() {
                    scratchpad.clear_just_shown();
                    let ctx = self.build_scratchpad_context();
                    let commands = scratchpad.on_pane_update(&ctx, &orphaned_tabs);
                    self.scratchpad = Some(scratchpad);
                    self.execute_scratchpad_commands(commands);
                }
            }
            Event::TabUpdate(tab_infos) => {
                eprintln!("TabUpdate: {:?}", tab_infos);
                if let Some(active_tab) = tab_infos.iter().find(|t| t.active) {
                    self.current_tab_position = active_tab.position;
                    self.are_floating_panes_visible = active_tab.are_floating_panes_visible;
                } else {
                    eprintln!("Warning: No active tab found");
                }
            }
            Event::ConfigWasWrittenToDisk => {
                eprintln!("ConfigWasWrittenToDisk: saving state and reloading plugin");
                if let Some(ref scratchpad) = self.scratchpad {
                    let state = scratchpad.persisted_state();
                    let ids = get_plugin_ids();
                    if let Err(e) = save_state(&state, ids.zellij_pid) {
                        eprintln!("Failed to save state: {}", e);
                    }
                    reload_plugin_with_id(ids.plugin_id);
                } else {
                    // No scratchpad manager, just reload
                    reload_plugin_with_id(get_plugin_ids().plugin_id);
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

        match parse_message(&payload) {
            Ok(message) => match self.handle_event(&message.event, message.args) {
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
