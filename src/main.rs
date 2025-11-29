use indexmap::IndexSet;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};

use zellij_tile::prelude::*;

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
    RegisterTab { name: String, tab: usize, pane_id: u32 },
    RegisterSession { name: String, pane_id: u32 },
}

#[derive(Default)]
struct State {
    // Pane tracking (from PaneUpdate events)
    pane_manifest: HashMap<usize, Vec<PaneInfo>>,

    // Current tab (from TabUpdate events)
    current_tab: usize,

    // Scratchpad configuration (from plugin load)
    scratchpad_configs: HashMap<String, ScratchpadConfig>,

    // Tab-scoped scratchpad state: (name, tab) -> pane_id
    tab_scratchpad_panes: HashMap<(String, usize), u32>,
    tab_pending_registrations: HashSet<(String, usize)>,

    // Session-scoped scratchpad state: name -> pane_id
    session_scratchpad_panes: HashMap<String, u32>,
    session_pending_registrations: HashSet<String>,

    // Unified focus history per tab (includes both scopes): tab -> names
    focused_history: HashMap<usize, IndexSet<String>>,
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
                write!(f, "Invalid scratchpad name '{}': must match [a-zA-Z0-9_-]+", name)
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
                // Format: register::tab::<tab>::<name>::<pane_id>
                let scope = args.get(1).ok_or_else(|| {
                    ParseError::InvalidArgs("register requires a scope (session or tab)".to_string())
                })?;

                match scope.as_str() {
                    "session" => {
                        let name = args.get(2).ok_or_else(|| {
                            ParseError::InvalidArgs("register::session requires a name".to_string())
                        })?;
                        let pane_id_str = args.get(3).ok_or_else(|| {
                            ParseError::InvalidArgs("register::session requires a pane_id".to_string())
                        })?;
                        let pane_id = pane_id_str.parse::<u32>().map_err(|e| {
                            ParseError::InvalidArgs(format!("Invalid pane_id '{}': {}", pane_id_str, e))
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
                        let tab_str = args.get(2).ok_or_else(|| {
                            ParseError::InvalidArgs("register::tab requires a tab index".to_string())
                        })?;
                        let tab = tab_str.parse::<usize>().map_err(|e| {
                            ParseError::InvalidArgs(format!("Invalid tab index '{}': {}", tab_str, e))
                        })?;
                        let name = args.get(3).ok_or_else(|| {
                            ParseError::InvalidArgs("register::tab requires a name".to_string())
                        })?;
                        let pane_id_str = args.get(4).ok_or_else(|| {
                            ParseError::InvalidArgs("register::tab requires a pane_id".to_string())
                        })?;
                        let pane_id = pane_id_str.parse::<u32>().map_err(|e| {
                            ParseError::InvalidArgs(format!("Invalid pane_id '{}': {}", pane_id_str, e))
                        })?;
                        if !is_valid_scratchpad_name(name) {
                            return Err(ParseError::InvalidScratchpadName(name.clone()));
                        }
                        Ok(ScratchpadAction::RegisterTab {
                            name: name.clone(),
                            tab,
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

    fn cleanup_closed_scratchpads(&mut self) {
        // Collect all pane IDs that currently exist
        let existing_pane_ids: HashSet<u32> = self
            .pane_manifest
            .values()
            .flatten()
            .map(|p| p.id)
            .collect();

        // Find scratchpads whose panes no longer exist
        let closed_scratchpads: Vec<String> = self
            .scratchpad_panes
            .iter()
            .filter(|(_, &pane_id)| !existing_pane_ids.contains(&pane_id))
            .map(|(name, _)| name.clone())
            .collect();

        // Remove closed scratchpads from tracking
        for name in closed_scratchpads {
            self.scratchpad_panes.remove(&name);
            self.focused_scratchpad_history.shift_remove(&name);
        }
    }

    fn get_hidden_floating_pane_ids(&self) -> HashSet<u32> {
        self.pane_manifest
            .values()
            .flatten()
            .filter(|p| p.is_floating && p.is_suppressed)
            .map(|p| p.id)
            .collect()
    }

    fn is_scratchpad_visible(&self, name: &str) -> bool {
        if let Some(&pane_id) = self.scratchpad_panes.get(name) {
            self.pane_manifest
                .values()
                .flatten()
                .any(|p| p.id == pane_id && !p.is_suppressed)
        } else {
            false
        }
    }

    fn get_focused_scratchpad(&self) -> Option<String> {
        let focused_pane_id = self
            .pane_manifest
            .values()
            .flatten()
            .find(|p| p.is_focused)?
            .id;

        self.scratchpad_panes
            .iter()
            .find(|(_, &pane_id)| pane_id == focused_pane_id)
            .map(|(name, _)| name.clone())
    }

    fn build_shim_command(&self, name: &str, config: &ScratchpadConfig) -> CommandToRun {
        // Build: sh -c 'zellij pipe "zellij-tools::scratchpad::register::<name>::$ZELLIJ_PANE_ID" && exec "$@"' _ <cmd> <args...>
        let mut args = vec![
            "-c".to_string(),
            format!(
                r#"zellij pipe "zellij-tools::scratchpad::register::{}::$ZELLIJ_PANE_ID" && exec "$@""#,
                name
            ),
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

        // If not yet registered, spawn the pane
        if !self.scratchpad_panes.contains_key(name) {
            if self.pending_registrations.contains(name) {
                return; // Already spawning
            }

            self.pending_registrations.insert(name.to_string());

            let cmd = self.build_shim_command(name, &config);
            let context = BTreeMap::new();
            open_command_pane_floating(cmd, None, context);
            return;
        }

        // Already registered - show the pane
        let pane_id = *self.scratchpad_panes.get(name).unwrap();

        // Capture which panes are currently hidden BEFORE showing
        let hidden_before = self.get_hidden_floating_pane_ids();

        // Show the scratchpad pane (this will show ALL floating panes as a side effect)
        show_pane_with_id(PaneId::Terminal(pane_id), true);

        // Re-hide all panes that were hidden before (except our scratchpad)
        for hidden_pane_id in hidden_before {
            if hidden_pane_id != pane_id {
                hide_pane_with_id(PaneId::Terminal(hidden_pane_id));
            }
        }

        // Update focus history
        self.focused_scratchpad_history.shift_remove(name);
        self.focused_scratchpad_history.insert(name.to_string());
    }

    fn handle_scratchpad_hide(&mut self, name: &str) {
        if let Some(&pane_id) = self.scratchpad_panes.get(name) {
            hide_pane_with_id(PaneId::Terminal(pane_id));
        }
        // Silent no-op if not registered
    }

    fn handle_scratchpad_close(&mut self, name: &str) {
        if let Some(pane_id) = self.scratchpad_panes.remove(name) {
            close_terminal_pane(pane_id);
            self.focused_scratchpad_history.shift_remove(name);
        }
        // Silent no-op if not registered
    }

    fn handle_scratchpad_register(&mut self, name: &str, pane_id: u32) {
        self.pending_registrations.remove(name);
        self.scratchpad_panes.insert(name.to_string(), pane_id);

        // Update focus history (newly spawned scratchpad is now focused)
        self.focused_scratchpad_history.shift_remove(name);
        self.focused_scratchpad_history.insert(name.to_string());

        // Re-hide any floating panes that should be hidden
        // (The newly spawned pane showing may have revealed other floating panes)
        let hidden_panes = self.get_hidden_floating_pane_ids();
        for hidden_pane_id in hidden_panes {
            if hidden_pane_id != pane_id {
                hide_pane_with_id(PaneId::Terminal(hidden_pane_id));
            }
        }
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
                    // No focused scratchpad - use last from history
                    match self.focused_scratchpad_history.last() {
                        Some(last) => last.clone(),
                        None => return, // No history - no-op
                    }
                }
            }
        };

        // Check if configured
        if !self.scratchpad_configs.contains_key(&target_name) {
            return; // Silent no-op for unknown scratchpad
        }

        // Toggle based on current visibility
        if self.is_scratchpad_visible(&target_name) {
            self.handle_scratchpad_hide(&target_name);
        } else {
            self.handle_scratchpad_show(&target_name);
        }
    }

    fn handle_scratchpad_action(&mut self, action: ScratchpadAction) {
        match action {
            ScratchpadAction::Toggle { name } => self.handle_scratchpad_toggle(name),
            ScratchpadAction::Show { name } => self.handle_scratchpad_show(&name),
            ScratchpadAction::Hide { name } => self.handle_scratchpad_hide(&name),
            ScratchpadAction::Close { name } => self.handle_scratchpad_close(&name),
            ScratchpadAction::Register { name, pane_id } => {
                self.handle_scratchpad_register(&name, pane_id)
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
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
        ]);

        subscribe(&[EventType::PaneUpdate]);

        // Parse scratchpad configuration from JSON
        if let Some(scratchpads_json) = configuration.get("scratchpads") {
            match serde_json::from_str::<HashMap<String, ScratchpadConfig>>(scratchpads_json) {
                Ok(configs) => {
                    // Validate all scratchpad names
                    for name in configs.keys() {
                        if !is_valid_scratchpad_name(name) {
                            eprintln!(
                                "Warning: Invalid scratchpad name '{}', skipping",
                                name
                            );
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
                self.pane_manifest = pane_manifest.panes;
                self.cleanup_closed_scratchpads();
                true
            }
            _ => false,
        }
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
