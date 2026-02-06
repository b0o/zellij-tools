use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use zellij_tile::prelude::*;

use zellij_tools::config::resolve_include_path;
use zellij_tools::focus::FocusTracker;
use zellij_tools::message::{parse_message, ParseError};
use zellij_tools::scratchpad::{
    parse_scratchpad_action, parse_scratchpads_kdl, ScratchpadCommand, ScratchpadContext,
    ScratchpadManager,
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
    focus_tracker: FocusTracker,

    // Raw include path from config (resolved after /host is mounted)
    raw_include: Option<String>,
    // User-provided config_dir override
    config_dir_override: Option<String>,
    // Resolved path to read from (with /host prefix)
    include_path: Option<PathBuf>,
    // Inline scratchpad config from plugin configuration (for merging)
    inline_scratchpads_kdl: Option<String>,
    // Whether we need to mount / after permissions are granted
    needs_host_mount: bool,
    // Last modified time of external config (for polling)
    config_last_modified: Option<std::time::SystemTime>,
    // Watch interval in milliseconds (None = disabled, Some(ms) = poll interval)
    watch_interval_ms: Option<u64>,
}

register_plugin!(State);

impl State {
    /// Load and merge inline + external configs
    fn load_merged_configs(
        &self,
    ) -> std::collections::HashMap<String, zellij_tools::scratchpad::ScratchpadConfig> {
        let mut configs = std::collections::HashMap::new();

        // First, parse inline config
        if let Some(ref inline_kdl) = self.inline_scratchpads_kdl {
            if let Ok(inline_configs) = parse_scratchpads_kdl(inline_kdl) {
                configs.extend(inline_configs);
            }
        }

        // Then, parse external file from /host and merge (external wins)
        if let Some(ref include_path) = self.include_path {
            if let Ok(contents) = std::fs::read_to_string(include_path) {
                if let Ok(external_configs) = Self::parse_external_config(&contents) {
                    configs.extend(external_configs);
                }
            }
        }

        configs
    }

    /// Parse external config file (expects `scratchpads { ... }` at top level)
    fn parse_external_config(
        contents: &str,
    ) -> Result<std::collections::HashMap<String, zellij_tools::scratchpad::ScratchpadConfig>, String>
    {
        use kdl::KdlDocument;

        let doc: KdlDocument = contents
            .parse()
            .map_err(|e| format!("KDL parse error: {}", e))?;

        // Find scratchpads node
        if let Some(scratchpads_node) = doc.get("scratchpads") {
            if let Some(children) = scratchpads_node.children() {
                // Convert children back to string and parse
                let scratchpads_kdl = children.to_string();
                parse_scratchpads_kdl(&scratchpads_kdl)
            } else {
                Ok(std::collections::HashMap::new())
            }
        } else {
            Ok(std::collections::HashMap::new())
        }
    }

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

    fn handle_event(&mut self, pipe_message: &PipeMessage) -> Result<(), ParseError> {
        let payload = pipe_message.payload.as_deref().unwrap_or("");
        let message = parse_message(payload)?;

        match message.event.as_str() {
            "focus-pane" => {
                if message.args.len() != 1 {
                    return Err(ParseError::InvalidArgs(format!(
                        "focus-pane requires 1 argument, got {}",
                        message.args.len()
                    )));
                }

                let pane_id = message.args[0]
                    .parse::<PaneId>()
                    .map_err(|e| ParseError::InvalidArgs(format!("Invalid pane ID: {}", e)))?;

                show_pane_with_id(pane_id, true);
                Ok(())
            }
            "scratchpad" => {
                let action = parse_scratchpad_action(&message.args)?;
                if let Some(mut scratchpad) = self.scratchpad.take() {
                    let ctx = self.build_scratchpad_context();
                    let commands = scratchpad.handle_action(action, &ctx);
                    self.scratchpad = Some(scratchpad);
                    self.execute_scratchpad_commands(commands);
                }
                Ok(())
            }
            "subscribe-focus" => {
                // Verify this came from a CLI pipe
                if !matches!(&pipe_message.source, PipeSource::Cli(_)) {
                    return Err(ParseError::InvalidArgs(
                        "subscribe-focus only works from CLI pipes".to_string(),
                    ));
                }

                // Use the pipe NAME (not the CLI pipe ID) for cli_pipe_output.
                // cli_pipe_output identifies pipes by name, not by the auto-assigned CLI ID.
                let pipe_name = pipe_message.name.clone();

                let pane_filter = if message.args.is_empty() {
                    None
                } else {
                    Some(message.args[0].parse::<u32>().map_err(|_| {
                        ParseError::InvalidArgs(format!("Invalid pane ID: {}", message.args[0]))
                    })?)
                };

                eprintln!(
                    "[zjt:subscribe] pipe_name={} pane_filter={:?}",
                    pipe_name, pane_filter
                );
                if let Some(event) = self.focus_tracker.subscribe(pipe_name.clone(), pane_filter) {
                    eprintln!(
                        "[zjt:subscribe] emitting initial state: {}",
                        event.to_json()
                    );
                    self.emit_focus_event(&pipe_name, &event.to_json());
                }
                eprintln!(
                    "[zjt:subscribe] has_subscribers={}",
                    self.focus_tracker.has_subscribers()
                );
                Ok(())
            }
            "unsubscribe-focus" => {
                if message.args.is_empty() {
                    return Err(ParseError::InvalidArgs(
                        "unsubscribe-focus requires pipe_id argument".to_string(),
                    ));
                }
                self.focus_tracker.unsubscribe(&message.args[0]);
                Ok(())
            }
            _ => Err(ParseError::UnknownEvent(message.event)),
        }
    }

    fn emit_focus_event(&self, pipe_id: &str, json: &str) {
        // Append newline so CLI can read line-by-line with BufReader::lines()
        cli_pipe_output(pipe_id, &format!("{}\n", json));
    }

    fn find_focused_pane(&self) -> Option<(u32, bool)> {
        for panes in self.pane_manifest.values() {
            for pane in panes {
                if pane.is_focused {
                    return Some((pane.id, pane.is_plugin));
                }
            }
        }
        None
    }
}

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
            PermissionType::ReadCliPipes,
            PermissionType::FullHdAccess,
        ]);

        subscribe(&[
            EventType::PaneUpdate,
            EventType::TabUpdate,
            EventType::HostFolderChanged,
            EventType::FailedToChangeHostFolder,
            EventType::PermissionRequestResult,
            EventType::Timer,
        ]);

        // Store inline scratchpads config for merging
        self.inline_scratchpads_kdl = configuration.get("scratchpads").cloned();

        // Store raw include path - will resolve after /host is mounted to /
        if let Some(include) = configuration.get("include") {
            self.raw_include = Some(include.clone());
            self.config_dir_override = configuration.get("config_dir").cloned();
            self.needs_host_mount = true;

            // Parse watch_ms option (default: 2000ms when include is set)
            self.watch_interval_ms = match configuration.get("watch_ms") {
                Some(val) if val == "false" || val == "0" => None,
                Some(val) => val.parse::<u64>().ok().or(Some(2000)),
                None => Some(2000),
            };
        }

        // Load inline configs immediately (external will load when HostFolderChanged arrives)
        if let Some(ref inline_kdl) = self.inline_scratchpads_kdl {
            if let Ok(configs) = parse_scratchpads_kdl(inline_kdl) {
                self.scratchpad = Some(ScratchpadManager::new(configs));
            }
        }
    }

    fn update(&mut self, event: Event) -> bool {
        let mut should_render = false;

        match event {
            Event::PaneUpdate(pane_manifest) => {
                self.pane_manifest = pane_manifest.panes;

                // Track focus changes and emit events
                let focused = self.find_focused_pane();
                let events = self.focus_tracker.on_focus_change(focused);
                if self.focus_tracker.has_subscribers() {
                    eprintln!(
                        "[zjt:pane_update] focused={:?} events_count={}",
                        focused,
                        events.len()
                    );
                }
                if !events.is_empty() {
                    should_render = true;
                }
                for (pipe_id, json) in &events {
                    eprintln!("[zjt:emit] pipe_id={} json={}", pipe_id, json);
                    self.emit_focus_event(pipe_id, json);
                }

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
                if let Some(active_tab) = tab_infos.iter().find(|t| t.active) {
                    self.current_tab_position = active_tab.position;
                    self.are_floating_panes_visible = active_tab.are_floating_panes_visible;
                }
            }
            Event::HostFolderChanged(_new_path) => {
                // Resolve include path now that /host is mounted to /
                if let Some(raw_include) = self.raw_include.take() {
                    let config_dir = self.config_dir_override.as_deref();
                    let resolved = resolve_include_path(&raw_include, config_dir);

                    // Convert to /host-prefixed path for reading
                    let host_path = PathBuf::from("/host")
                        .join(resolved.strip_prefix("/").unwrap_or(&resolved));
                    self.include_path = Some(host_path);
                }

                if let Some(ref include_path) = self.include_path {
                    // Store the initial mtime for polling
                    if let Ok(metadata) = std::fs::metadata(include_path) {
                        self.config_last_modified = metadata.modified().ok();
                    }

                    // Load the config
                    let configs = self.load_merged_configs();

                    if let Some(ref mut scratchpad) = self.scratchpad {
                        let commands = scratchpad.reconcile_config(configs);
                        self.execute_scratchpad_commands(commands);
                    } else if !configs.is_empty() {
                        self.scratchpad = Some(ScratchpadManager::new(configs));
                    }

                    // Start polling timer if watching is enabled
                    if let Some(interval_ms) = self.watch_interval_ms {
                        set_timeout(interval_ms as f64 / 1000.0);
                    }
                }
            }
            Event::FailedToChangeHostFolder(_err) => {
                // Could not mount root filesystem - external config won't be available
            }
            Event::PermissionRequestResult(result) => {
                // Mount root filesystem so we can access /proc/self/environ for config resolution
                if result == PermissionStatus::Granted && self.needs_host_mount {
                    change_host_folder(PathBuf::from("/"));
                    self.needs_host_mount = false;
                }
            }
            Event::Timer(_elapsed) => {
                // Check if config file has changed (only if watching is enabled)
                if let (Some(ref include_path), Some(interval_ms)) =
                    (&self.include_path, self.watch_interval_ms)
                {
                    if let Ok(metadata) = std::fs::metadata(include_path) {
                        let current_mtime = metadata.modified().ok();
                        if current_mtime != self.config_last_modified {
                            self.config_last_modified = current_mtime;

                            // Reload config
                            let new_configs = self.load_merged_configs();

                            if let Some(ref mut scratchpad) = self.scratchpad {
                                let commands = scratchpad.reconcile_config(new_configs);
                                self.execute_scratchpad_commands(commands);
                            } else if !new_configs.is_empty() {
                                self.scratchpad = Some(ScratchpadManager::new(new_configs));
                            }
                        }
                    }

                    // Schedule next poll
                    set_timeout(interval_ms as f64 / 1000.0);
                }
            }
            _ => (),
        };
        should_render
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        eprintln!(
            "[zjt:pipe] source={:?} name={:?} payload={:?}",
            pipe_message.source, pipe_message.name, pipe_message.payload
        );
        match self.handle_event(&pipe_message) {
            Ok(()) => {
                eprintln!("[zjt:pipe] handled OK");
                true
            }
            Err(e) => {
                eprintln!("[zjt:pipe] error: {}", e);
                false
            }
        }
    }
}
