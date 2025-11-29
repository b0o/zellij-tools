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
        // Collect all pane IDs that still exist in the manifest
        let existing_pane_ids: HashSet<u32> = self
            .pane_manifest
            .values()
            .flatten()
            .map(|p| p.id)
            .collect();

        // Clean up tab-scoped scratchpads whose panes no longer exist
        let closed_tab_scratchpads: Vec<(String, usize)> = self
            .tab_scratchpad_panes
            .iter()
            .filter(|(_, &pane_id)| !existing_pane_ids.contains(&pane_id))
            .map(|((name, tab), _)| (name.clone(), *tab))
            .collect();

        for (name, tab) in closed_tab_scratchpads {
            self.tab_scratchpad_panes.remove(&(name.clone(), tab));
            if let Some(history) = self.focused_history.get_mut(&tab) {
                history.shift_remove(&name);
            }
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
            // Remove from ALL tab histories
            for history in self.focused_history.values_mut() {
                history.shift_remove(&name);
            }
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
        let exited_tab_scratchpads: Vec<((String, usize), u32)> = self
            .tab_scratchpad_panes
            .iter()
            .filter(|(_, &pane_id)| exited_pane_ids.contains(&pane_id))
            .map(|((name, tab), &pane_id)| ((name.clone(), *tab), pane_id))
            .collect();

        for ((name, tab), pane_id) in exited_tab_scratchpads {
            close_terminal_pane(pane_id);
            self.tab_scratchpad_panes.remove(&(name.clone(), tab));
            if let Some(history) = self.focused_history.get_mut(&tab) {
                history.shift_remove(&name);
            }
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
            // Remove from ALL tab histories
            for history in self.focused_history.values_mut() {
                history.shift_remove(&name);
            }
        }
    }

    fn get_pane_tab(&self, pane_id: u32) -> Option<usize> {
        self.pane_manifest
            .iter()
            .find(|(_, panes)| panes.iter().any(|p| p.id == pane_id))
            .map(|(tab, _)| *tab)
    }

    fn get_scratchpad_scope(&self, name: &str) -> Option<ScratchpadScope> {
        self.scratchpad_configs.get(name).map(|c| c.scope)
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
        let scope = self.get_scratchpad_scope(name);
        let pane_id = match scope {
            Some(ScratchpadScope::Session) => self.session_scratchpad_panes.get(name).copied(),
            Some(ScratchpadScope::Tab) => self
                .tab_scratchpad_panes
                .get(&(name.to_string(), self.current_tab))
                .copied(),
            None => return false,
        };

        let Some(pane_id) = pane_id else {
            return false;
        };

        // For tab-scoped: only check panes on current tab
        // For session-scoped: check all tabs but verify it's on current tab
        let panes_to_check: Box<dyn Iterator<Item = &PaneInfo>> = match scope {
            Some(ScratchpadScope::Tab) => {
                // Only check current tab's panes for tab-scoped scratchpads
                Box::new(
                    self.pane_manifest
                        .get(&self.current_tab)
                        .into_iter()
                        .flatten(),
                )
            }
            Some(ScratchpadScope::Session) => {
                // Check all tabs for session-scoped
                Box::new(self.pane_manifest.values().flatten())
            }
            None => return false,
        };

        panes_to_check.into_iter().any(|p| {
            p.id == pane_id
                && !p.is_suppressed
                && !p.exited
                && !p.is_held
                // For session-scoped, also verify it's on the current tab
                && (scope != Some(ScratchpadScope::Session)
                    || self.get_pane_tab(pane_id) == Some(self.current_tab))
        })
    }

    fn get_focused_scratchpad(&self) -> Option<String> {
        let focused_pane_id = self
            .pane_manifest
            .values()
            .flatten()
            .find(|p| p.is_focused)?
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

        // Check tab-scoped scratchpads (only on current tab)
        self.tab_scratchpad_panes
            .iter()
            .filter(|((_, tab), _)| *tab == self.current_tab)
            .find(|(_, &pane_id)| pane_id == focused_pane_id)
            .map(|((name, _), _)| name.clone())
    }

    fn build_shim_command(&self, name: &str, config: &ScratchpadConfig) -> CommandToRun {
        let register_msg = match config.scope {
            ScratchpadScope::Session => {
                format!(
                    r#"zellij pipe "zellij-tools::scratchpad::register::session::{}::$ZELLIJ_PANE_ID""#,
                    name
                )
            }
            ScratchpadScope::Tab => {
                format!(
                    r#"zellij pipe "zellij-tools::scratchpad::register::tab::{}::{}::$ZELLIJ_PANE_ID""#,
                    self.current_tab, name
                )
            }
        };

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
        let key = (name.to_string(), self.current_tab);

        // If not yet registered, spawn the pane
        if !self.tab_scratchpad_panes.contains_key(&key) {
            if self.tab_pending_registrations.contains(&key) {
                return; // Already spawning
            }

            self.tab_pending_registrations.insert(key);

            let cmd = self.build_shim_command(name, config);
            let context = BTreeMap::new();
            open_command_pane_floating(cmd, None, context);
            return;
        }

        // Already registered - show the pane
        let pane_id = *self.tab_scratchpad_panes.get(&(name.to_string(), self.current_tab)).unwrap();
        self.show_scratchpad_pane(name, pane_id);
    }

    fn handle_session_scratchpad_show(&mut self, name: &str, config: &ScratchpadConfig) {
        // If not yet registered, spawn the pane
        if !self.session_scratchpad_panes.contains_key(name) {
            if self.session_pending_registrations.contains(name) {
                return; // Already spawning
            }

            self.session_pending_registrations.insert(name.to_string());

            let cmd = self.build_shim_command(name, config);
            let context = BTreeMap::new();
            open_command_pane_floating(cmd, None, context);
            return;
        }

        // Already registered - check if on current tab
        let pane_id = *self.session_scratchpad_panes.get(name).unwrap();
        let pane_tab = self.get_pane_tab(pane_id);

        // If on different tab, move it to current tab
        if pane_tab != Some(self.current_tab) {
            break_panes_to_tab_with_index(&[PaneId::Terminal(pane_id)], self.current_tab, false);
        }

        self.show_scratchpad_pane(name, pane_id);
    }

    fn show_scratchpad_pane(&mut self, name: &str, pane_id: u32) {
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

        // Update focus history for current tab
        let history = self.focused_history.entry(self.current_tab).or_default();
        history.shift_remove(name);
        history.insert(name.to_string());
    }

    fn handle_scratchpad_hide(&mut self, name: &str) {
        let pane_id = match self.get_scratchpad_scope(name) {
            Some(ScratchpadScope::Session) => self.session_scratchpad_panes.get(name).copied(),
            Some(ScratchpadScope::Tab) => self
                .tab_scratchpad_panes
                .get(&(name.to_string(), self.current_tab))
                .copied(),
            None => return, // Unknown scratchpad
        };

        if let Some(pane_id) = pane_id {
            hide_pane_with_id(PaneId::Terminal(pane_id));
        }
        // Silent no-op if not registered
    }

    fn handle_scratchpad_close(&mut self, name: &str) {
        match self.get_scratchpad_scope(name) {
            Some(ScratchpadScope::Session) => {
                if let Some(pane_id) = self.session_scratchpad_panes.remove(name) {
                    close_terminal_pane(pane_id);
                    // Remove from all tab histories
                    for history in self.focused_history.values_mut() {
                        history.shift_remove(name);
                    }
                }
            }
            Some(ScratchpadScope::Tab) => {
                let key = (name.to_string(), self.current_tab);
                if let Some(pane_id) = self.tab_scratchpad_panes.remove(&key) {
                    close_terminal_pane(pane_id);
                    if let Some(history) = self.focused_history.get_mut(&self.current_tab) {
                        history.shift_remove(name);
                    }
                }
            }
            None => {} // Unknown scratchpad - no-op
        }
    }

    fn handle_scratchpad_register_tab(&mut self, name: &str, tab: usize, pane_id: u32) {
        self.tab_pending_registrations.remove(&(name.to_string(), tab));
        self.tab_scratchpad_panes.insert((name.to_string(), tab), pane_id);

        // Update focus history (newly spawned scratchpad is now focused)
        let history = self.focused_history.entry(tab).or_default();
        history.shift_remove(name);
        history.insert(name.to_string());

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
        self.session_scratchpad_panes.insert(name.to_string(), pane_id);

        // Update focus history for current tab
        let history = self.focused_history.entry(self.current_tab).or_default();
        history.shift_remove(name);
        history.insert(name.to_string());

        // Re-hide any floating panes that should be hidden
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
                    // No focused scratchpad - use last from current tab's history
                    match self.focused_history.get(&self.current_tab).and_then(|h| h.last()) {
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
            ScratchpadAction::RegisterTab { name, tab, pane_id } => {
                self.handle_scratchpad_register_tab(&name, tab, pane_id)
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
                self.close_exited_scratchpads();
                self.cleanup_closed_scratchpads();
                true
            }
            Event::TabUpdate(tab_infos) => {
                // Find the active tab
                if let Some(active_tab) = tab_infos.iter().find(|t| t.active) {
                    self.current_tab = active_tab.position;
                }
                false // No UI update needed
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
