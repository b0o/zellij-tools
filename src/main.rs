use std::collections::{BTreeMap, HashMap};

use zellij_tile::prelude::*;

use zellij_tools::message::{parse_message, ParseError};
use zellij_tools::scratchpad::{
    is_valid_scratchpad_name, parse_scratchpad_action, ScratchpadCommand, ScratchpadConfig,
    ScratchpadContext, ScratchpadManager,
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

        subscribe(&[EventType::PaneUpdate, EventType::TabUpdate]);

        // Parse scratchpad configuration from JSON
        if let Some(scratchpads_json) = configuration.get("scratchpads") {
            match serde_json::from_str::<HashMap<String, ScratchpadConfig>>(scratchpads_json) {
                Ok(configs) => {
                    // Validate and filter scratchpad names
                    for name in configs.keys() {
                        if !is_valid_scratchpad_name(name) {
                            eprintln!("Warning: Invalid scratchpad name '{}', skipping", name);
                        }
                    }
                    let valid_configs: HashMap<String, ScratchpadConfig> = configs
                        .into_iter()
                        .filter(|(name, _)| is_valid_scratchpad_name(name))
                        .collect();
                    self.scratchpad = Some(ScratchpadManager::new(valid_configs));
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
