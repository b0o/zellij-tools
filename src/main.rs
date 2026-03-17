use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use zellij_tile::prelude::*;

use zellij_tools::config::resolve_include_path;
use zellij_tools::events::{
    Event as StreamEvent, EventStream, PaneInfo as EventPaneInfo, SubscribeMode,
    TabInfo as EventTabInfo,
};
use zellij_tools::focus::{parse_focus_tab_target, FocusTabTarget};
use zellij_tools::message::{parse_message, ParseError};
use zellij_tools::scratchpad::{
    parse_scratchpad_action, parse_scratchpads_kdl, ScratchpadCommand, ScratchpadContext,
    ScratchpadListQuery, ScratchpadManager,
};
use zellij_tools::stable_tabs::StableTabTracker;
use zellij_tools::tree;

#[derive(Default)]
struct State {
    // Pane tracking (from PaneUpdate events)
    pane_manifest: HashMap<usize, Vec<PaneInfo>>,

    // Tab tracking (from TabUpdate events)
    tab_infos: Vec<TabInfo>,
    current_tab_position: usize,
    are_floating_panes_visible: bool,

    // Managers
    tab_tracker: StableTabTracker,
    scratchpad: Option<ScratchpadManager>,
    event_stream: EventStream,

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
    fn current_event_context(&self) -> (Vec<EventPaneInfo>, Vec<EventTabInfo>) {
        let event_panes: Vec<EventPaneInfo> = self
            .pane_manifest
            .iter()
            .flat_map(|(&tab_pos, panes)| {
                panes.iter().map(move |p| EventPaneInfo {
                    id: p.id,
                    is_focused: p.is_focused,
                    is_floating: p.is_floating,
                    is_suppressed: p.is_suppressed,
                    is_plugin: p.is_plugin,
                    tab_position: tab_pos,
                    title: p.title.clone(),
                    terminal_command: p.terminal_command.clone(),
                    plugin_url: p.plugin_url.clone(),
                })
            })
            .collect();
        let event_tabs: Vec<EventTabInfo> = self
            .tab_infos
            .iter()
            .map(|t| {
                let stable_id = self
                    .tab_tracker
                    .get_stable_id(t.position)
                    .unwrap_or(t.position as u64);
                EventTabInfo {
                    stable_id,
                    position: t.position,
                    name: t.name.clone(),
                    active: t.active,
                }
            })
            .collect();
        (event_panes, event_tabs)
    }

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
        let (viewport_cols, viewport_rows) = self
            .tab_infos
            .iter()
            .find(|t| t.active)
            .map(|t| (t.viewport_columns, t.viewport_rows))
            .unwrap_or((0, 0));

        ScratchpadContext {
            pane_manifest: &self.pane_manifest,
            current_tab_position: self.current_tab_position,
            current_stable_tab_id: self.tab_tracker.get_stable_id(self.current_tab_position),
            are_floating_panes_visible: self.are_floating_panes_visible,
            stable_tab_to_position: &self.tab_tracker.stable_tab_to_position,
            viewport_cols,
            viewport_rows,
        }
    }

    fn execute_scratchpad_commands(&self, commands: Vec<ScratchpadCommand>) {
        for cmd in commands {
            match cmd {
                ScratchpadCommand::OpenFloating {
                    command,
                    coordinates,
                } => {
                    let coords = FloatingPaneCoordinates::new(
                        coordinates.x,
                        coordinates.y,
                        coordinates.width,
                        coordinates.height,
                        None,
                    );
                    open_command_pane_floating(command, coords, BTreeMap::new());
                }
                ScratchpadCommand::ShowPane {
                    pane_id,
                    coordinates,
                } => {
                    if let Some(resolved) = coordinates {
                        let coords = FloatingPaneCoordinates::new(
                            resolved.x,
                            resolved.y,
                            resolved.width,
                            resolved.height,
                            None,
                        );
                        if let Some(coords) = coords {
                            change_floating_panes_coordinates(vec![(
                                PaneId::Terminal(pane_id),
                                coords,
                            )]);
                        }
                    }
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
                ScratchpadCommand::RenamePane { pane_id, name } => {
                    rename_terminal_pane(pane_id, &name);
                }
            }
        }
    }

    fn handle_event(&mut self, pipe_message: &PipeMessage) -> Result<(), ParseError> {
        let payload = pipe_message.payload.as_deref().unwrap_or("");
        let message = parse_message(payload)?;

        match message.event {
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

                // If the pane is a scratchpad, use show logic to ensure correct size/position
                if let PaneId::Terminal(terminal_id) = pane_id {
                    if let Some(mut scratchpad) = self.scratchpad.take() {
                        let ctx = self.build_scratchpad_context();
                        let result = scratchpad.handle_focus_pane(terminal_id, &ctx);
                        self.scratchpad = Some(scratchpad);
                        if let Some(commands) = result {
                            self.execute_scratchpad_commands(commands);
                            return Ok(());
                        }
                    }
                }

                show_pane_with_id(pane_id, true);
                Ok(())
            }
            "focus-tab" => {
                let tab_index = match parse_focus_tab_target(&message.args)? {
                    FocusTabTarget::Position(position) => position,
                    FocusTabTarget::StableId(stable_id) => {
                        let position = self
                            .tab_tracker
                            .stable_tab_to_position
                            .get(&stable_id)
                            .copied()
                            .ok_or_else(|| {
                                ParseError::InvalidArgs(format!(
                                    "No tab found for stable ID {}",
                                    stable_id
                                ))
                            })?;
                        u32::try_from(position).map_err(|_| {
                            ParseError::InvalidArgs(format!(
                                "Tab position {} does not fit in u32",
                                position
                            ))
                        })?
                    }
                };
                go_to_tab(tab_index);
                Ok(())
            }
            "scratchpad" if message.args.first().copied() == Some("list") => {
                let cli_pipe_id = match &pipe_message.source {
                    PipeSource::Cli(id) => id.clone(),
                    _ => {
                        return Err(ParseError::InvalidArgs(
                            "scratchpad list only works from CLI pipes".to_string(),
                        ))
                    }
                };

                let rest = &message.args[1..];
                let mut full = false;
                let mut tab_id: Option<u64> = None;
                let mut names = Vec::new();

                for &arg in rest {
                    if arg == "full" {
                        full = true;
                    } else if let Some(id_str) = arg.strip_prefix("tab=") {
                        tab_id = id_str.parse().ok();
                    } else {
                        names.push(arg.to_string());
                    }
                }

                let query = ScratchpadListQuery {
                    names,
                    tab_id,
                    full,
                };

                let entries = if let Some(ref scratchpad) = self.scratchpad {
                    scratchpad.list(
                        &query,
                        &self.pane_manifest,
                        &self.tab_tracker.stable_tab_to_position,
                    )
                } else {
                    Vec::new()
                };

                let json = serde_json::to_string(&entries).unwrap_or_default();
                self.emit_event(&cli_pipe_id, &json);
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
            "subscribe" => {
                let cli_pipe_id = match &pipe_message.source {
                    PipeSource::Cli(id) => id.clone(),
                    _ => {
                        return Err(ParseError::InvalidArgs(
                            "subscribe only works from CLI pipes".to_string(),
                        ))
                    }
                };

                let mode = if message.args.first().copied() == Some("full") {
                    SubscribeMode::Full
                } else {
                    SubscribeMode::Compact
                };

                self.event_stream
                    .subscribe_pending(cli_pipe_id.clone(), mode);

                // Send ACK so CLI knows we're alive
                self.emit_event(&cli_pipe_id, &StreamEvent::Ack {}.to_json());
                Ok(())
            }
            "unsubscribe" => {
                if message.args.is_empty() {
                    return Err(ParseError::InvalidArgs(
                        "unsubscribe requires pipe_id argument".to_string(),
                    ));
                }
                self.event_stream.unsubscribe(message.args[0]);
                Ok(())
            }
            "tree" => {
                let cli_pipe_id = match &pipe_message.source {
                    PipeSource::Cli(id) => id.clone(),
                    _ => {
                        return Err(ParseError::InvalidArgs(
                            "tree only works from CLI pipes".to_string(),
                        ))
                    }
                };

                let session_tree =
                    tree::build_tree(&self.tab_infos, &self.pane_manifest, &self.tab_tracker);
                let json = serde_json::to_string(&session_tree).unwrap_or_default();
                self.emit_event(&cli_pipe_id, &json);
                Ok(())
            }
            _ => Err(ParseError::UnknownEvent(message.event.to_string())),
        }
    }

    fn emit_event(&self, pipe_id: &str, json: &str) {
        // Append newline so CLI can read line-by-line with BufReader::lines()
        cli_pipe_output(pipe_id, &format!("{}\n", json));
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
        match event {
            Event::PaneUpdate(pane_manifest) => {
                self.pane_manifest = pane_manifest.panes;

                if self.event_stream.has_subscribers() {
                    // Convert pane manifest into EventStream's PaneInfo format
                    // and emit any detected changes (focus, open, close).
                    let event_panes: Vec<EventPaneInfo> = self
                        .pane_manifest
                        .iter()
                        .flat_map(|(&tab_pos, panes)| {
                            panes.iter().map(move |p| EventPaneInfo {
                                id: p.id,
                                is_focused: p.is_focused,
                                is_floating: p.is_floating,
                                is_suppressed: p.is_suppressed,
                                is_plugin: p.is_plugin,
                                tab_position: tab_pos,
                                title: p.title.clone(),
                                terminal_command: p.terminal_command.clone(),
                                plugin_url: p.plugin_url.clone(),
                            })
                        })
                        .collect();

                    let events = self
                        .event_stream
                        .on_pane_update(&event_panes, self.current_tab_position);
                    for (pipe_id, json) in &events {
                        self.emit_event(pipe_id, json);
                    }
                } else {
                    // No subscribers — still update internal state (focus, known panes)
                    // but skip expensive String clones for title/command/url fields.
                    let cheap_panes: Vec<EventPaneInfo> = self
                        .pane_manifest
                        .iter()
                        .flat_map(|(&tab_pos, panes)| {
                            panes.iter().map(move |p| EventPaneInfo {
                                id: p.id,
                                is_focused: p.is_focused,
                                is_floating: p.is_floating,
                                is_suppressed: p.is_suppressed,
                                is_plugin: p.is_plugin,
                                tab_position: tab_pos,
                                title: String::new(),
                                terminal_command: None,
                                plugin_url: None,
                            })
                        })
                        .collect();
                    self.event_stream
                        .update_pane_state(&cheap_panes, self.current_tab_position);
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

                if self.event_stream.has_subscribers() {
                    // Emit tab events (focus, create, close, move)
                    let event_tabs: Vec<EventTabInfo> = tab_infos
                        .iter()
                        .map(|t| {
                            let stable_id = self
                                .tab_tracker
                                .get_stable_id(t.position)
                                .unwrap_or(t.position as u64);
                            EventTabInfo {
                                stable_id,
                                position: t.position,
                                name: t.name.clone(),
                                active: t.active,
                            }
                        })
                        .collect();
                    let events = self.event_stream.on_tab_update(&event_tabs);
                    for (pipe_id, json) in &events {
                        self.emit_event(pipe_id, json);
                    }
                } else {
                    // No subscribers — still update internal state (active tab, known tabs)
                    // but skip expensive String clones for tab names.
                    let cheap_tabs: Vec<EventTabInfo> = tab_infos
                        .iter()
                        .map(|t| {
                            let stable_id = self
                                .tab_tracker
                                .get_stable_id(t.position)
                                .unwrap_or(t.position as u64);
                            EventTabInfo {
                                stable_id,
                                position: t.position,
                                name: String::new(),
                                active: t.active,
                            }
                        })
                        .collect();
                    self.event_stream.update_tab_state(&cheap_tabs);
                }

                self.tab_infos = tab_infos;
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
        false
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        let payload = pipe_message.payload.as_deref().unwrap_or("");

        if payload.is_empty() || payload.trim().is_empty() {
            // Heartbeat from a CLI subscriber — record liveness and prune stale ones.
            if let PipeSource::Cli(ref id) = pipe_message.source {
                self.event_stream.record_heartbeat(id);
                // Prune every 20th tick; subscribers silent for 20+ ticks (~10s at 500ms) are stale
                if self.event_stream.heartbeat_counter() % 20 == 0 {
                    self.event_stream.prune_stale_subscribers(20);
                }
            }
            return false;
        }

        match self.handle_event(&pipe_message) {
            Ok(()) => true,
            Err(ParseError::WrongPlugin | ParseError::InvalidFormat) => {
                if let PipeSource::Cli(ref id) = pipe_message.source {
                    if self.event_stream.is_pending(id) {
                        match self.event_stream.initialize_subscriber(id, payload.trim()) {
                            Ok(()) => {
                                self.emit_event(id, &StreamEvent::InitAck {}.to_json());

                                let (event_panes, event_tabs) = self.current_event_context();
                                let mode = self
                                    .event_stream
                                    .subscriber_mode(id)
                                    .unwrap_or(SubscribeMode::Compact);
                                for event in self.event_stream.initial_events_for(id) {
                                    let json = match mode {
                                        SubscribeMode::Compact => event.to_json(),
                                        SubscribeMode::Full => {
                                            event.to_full_json(&event_panes, &event_tabs)
                                        }
                                    };
                                    self.emit_event(id, &json);
                                }
                            }
                            Err(err) => {
                                self.emit_event(
                                    id,
                                    &StreamEvent::InitError {
                                        message: err.message(),
                                    }
                                    .to_json(),
                                );
                            }
                        }
                        return true;
                    }
                }

                // Silently ignore messages from other plugins (zjstatus, etc.)
                // and keybind payloads that don't match our format.
                false
            }
            Err(e) => {
                eprintln!("Error handling event: {}", e);
                false
            }
        }
    }
}
