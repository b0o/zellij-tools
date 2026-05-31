use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
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
    load_state, parse_scratchpad_action, parse_scratchpads_kdl, save_state, ScratchpadCommand,
    ScratchpadConfig, ScratchpadContext, ScratchpadListQuery, ScratchpadManager,
};
use zellij_tools::tree;

const CLIENT_LIST_REFRESH_SECONDS: f64 = 2.0;

#[derive(Default)]
struct State {
    // Pane tracking (from PaneUpdate events)
    pane_manifest: HashMap<usize, Vec<PaneInfo>>,

    // Tab tracking (from TabUpdate events)
    tab_infos: Vec<TabInfo>,
    current_tab_position: usize,
    are_floating_panes_visible: bool,

    // Tab ID maps (rebuilt from TabUpdate events using Zellij's native tab_id)
    tab_id_to_position: HashMap<usize, usize>,
    position_to_tab_id: HashMap<usize, usize>,

    // Managers
    scratchpad: Option<ScratchpadManager>,
    event_stream: EventStream,

    // Zellij server identity, used for cross-client scratchpad state.
    zellij_pid: Option<u32>,
    own_client_id: Option<ClientId>,
    own_client_connected: bool,

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
    // Last time external config was checked.
    config_last_checked: Option<std::time::SystemTime>,
    // Watch interval in milliseconds (None = disabled, Some(ms) = poll interval)
    watch_interval_ms: Option<u64>,
}

register_plugin!(State);

impl State {
    fn claim_file_path(&self, pipe_name: &str) -> Option<PathBuf> {
        let zellij_pid = self.zellij_pid?;
        let safe_name: String = pipe_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        Some(PathBuf::from(format!(
            "/tmp/zellij-tools-{}-pipe-{}",
            zellij_pid, safe_name
        )))
    }

    fn claim_cli_pipe(&self, pipe_message: &PipeMessage) -> bool {
        if !matches!(pipe_message.source, PipeSource::Cli(_)) {
            return true;
        }

        if !self.own_client_connected {
            return false;
        }

        let Some(path) = self.claim_file_path(&pipe_message.name) else {
            return true;
        };

        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .is_ok()
    }

    fn refresh_client_list(&self) {
        list_clients();
    }

    fn schedule_timer(&self) {
        let seconds = self
            .watch_interval_ms
            .map(|ms| (ms as f64 / 1000.0).min(CLIENT_LIST_REFRESH_SECONDS))
            .unwrap_or(CLIENT_LIST_REFRESH_SECONDS);
        set_timeout(seconds);
    }

    fn sync_scratchpad_state_from_disk(&mut self) {
        let Some(zellij_pid) = self.zellij_pid else {
            return;
        };
        let Some(ref mut scratchpad) = self.scratchpad else {
            return;
        };
        if let Some(state) = load_state(zellij_pid) {
            scratchpad.replace_persisted_state(state);
        }
    }

    fn save_scratchpad_state_to_disk(&self) {
        let Some(zellij_pid) = self.zellij_pid else {
            return;
        };
        let Some(ref scratchpad) = self.scratchpad else {
            return;
        };
        if let Err(err) = save_state(&scratchpad.persisted_state(), zellij_pid) {
            eprintln!("Failed to save scratchpad state: {}", err);
        }
    }

    fn new_scratchpad_manager(
        &self,
        configs: HashMap<String, ScratchpadConfig>,
    ) -> ScratchpadManager {
        let mut manager = ScratchpadManager::new(configs);
        if let Some(zellij_pid) = self.zellij_pid {
            if let Some(state) = load_state(zellij_pid) {
                manager.restore_state(state);
            }
        }
        manager
    }

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
            .map(|t| EventTabInfo {
                tab_id: t.tab_id,
                position: t.position,
                name: t.name.clone(),
                active: t.active,
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
            current_tab_id: self
                .position_to_tab_id
                .get(&self.current_tab_position)
                .copied(),
            are_floating_panes_visible: self.are_floating_panes_visible,
            tab_id_to_position: &self.tab_id_to_position,
            viewport_cols,
            viewport_rows,
        }
    }

    fn execute_scratchpad_commands(&mut self, commands: Vec<ScratchpadCommand>) {
        let should_save = !commands.is_empty();
        for cmd in commands {
            match cmd {
                ScratchpadCommand::OpenFloating {
                    command,
                    coordinates,
                    name,
                    tab_id,
                } => {
                    let coords = FloatingPaneCoordinates::new(
                        coordinates.x,
                        coordinates.y,
                        coordinates.width,
                        coordinates.height,
                        None,
                        None,
                    );
                    let opened = open_command_pane_floating(command, coords, BTreeMap::new());
                    if let Some(PaneId::Terminal(pane_id)) = opened {
                        if let Some(ref mut mgr) = self.scratchpad {
                            let register_cmds = mgr.register_pane(&name, tab_id, pane_id);
                            // register_pane only returns RenamePane commands, execute them directly
                            for register_cmd in register_cmds {
                                if let ScratchpadCommand::RenamePane {
                                    pane_id,
                                    name: title,
                                } = register_cmd
                                {
                                    rename_terminal_pane(pane_id, &title);
                                }
                            }
                        }
                    }
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
                            None,
                        );
                        if let Some(coords) = coords {
                            change_floating_panes_coordinates(vec![(
                                PaneId::Terminal(pane_id),
                                coords,
                            )]);
                        }
                    }
                    show_pane_with_id(PaneId::Terminal(pane_id), true, true);
                }
                ScratchpadCommand::HidePane { pane_id } => {
                    hide_pane_with_id(PaneId::Terminal(pane_id));
                }
                ScratchpadCommand::ClosePane { pane_id } => {
                    close_terminal_pane(pane_id);
                }
                ScratchpadCommand::RenamePane { pane_id, name } => {
                    rename_terminal_pane(pane_id, &name);
                }
            }
        }
        if should_save {
            self.save_scratchpad_state_to_disk();
        }
    }

    fn handle_event(&mut self, pipe_message: &PipeMessage) -> Result<(), ParseError> {
        let payload = pipe_message.payload.as_deref().unwrap_or("");
        let message = parse_message(payload)?;

        if !self.claim_cli_pipe(pipe_message) {
            return Ok(());
        }

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
                    self.sync_scratchpad_state_from_disk();
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

                show_pane_with_id(pane_id, true, true);
                Ok(())
            }
            "focus-tab" => {
                let tab_index = match parse_focus_tab_target(&message.args)? {
                    FocusTabTarget::Position(position) => position,
                    FocusTabTarget::TabId(tab_id) => {
                        let position = self
                            .tab_infos
                            .iter()
                            .find(|t| t.tab_id == tab_id)
                            .map(|t| t.position)
                            .ok_or_else(|| {
                                ParseError::InvalidArgs(format!(
                                    "No tab found for tab ID {}",
                                    tab_id
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
                let mut tab_id: Option<usize> = None;
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

                self.sync_scratchpad_state_from_disk();
                let entries = if let Some(ref scratchpad) = self.scratchpad {
                    scratchpad.list(&query, &self.pane_manifest, &self.tab_id_to_position)
                } else {
                    Vec::new()
                };

                let json = serde_json::to_string(&entries).unwrap_or_default();
                self.emit_event(&cli_pipe_id, &json);
                Ok(())
            }
            "scratchpad" => {
                let action = parse_scratchpad_action(&message.args)?;
                self.sync_scratchpad_state_from_disk();
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

                let session_tree = tree::build_tree(&self.tab_infos, &self.pane_manifest);
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
        let plugin_ids = get_plugin_ids();
        self.zellij_pid = Some(plugin_ids.zellij_pid);
        self.own_client_id = Some(plugin_ids.client_id);
        self.own_client_connected = true;

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
            EventType::ListClients,
        ]);

        self.refresh_client_list();
        self.schedule_timer();

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
                self.scratchpad = Some(self.new_scratchpad_manager(configs));
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

                // Detect orphaned tabs by comparing current tab_id set with pane manifest keys
                // Tabs that exist in tab_id_to_position but not in pane_manifest are orphaned
                let current_tab_positions: std::collections::HashSet<usize> =
                    self.pane_manifest.keys().copied().collect();
                let orphaned_tabs: std::collections::HashSet<usize> = self
                    .tab_id_to_position
                    .iter()
                    .filter(|(_, pos)| !current_tab_positions.contains(pos))
                    .map(|(tab_id, _)| *tab_id)
                    .collect();

                // Update scratchpad manager
                self.sync_scratchpad_state_from_disk();
                if let Some(mut scratchpad) = self.scratchpad.take() {
                    scratchpad.clear_just_shown();
                    let ctx = self.build_scratchpad_context();
                    let commands = scratchpad.on_pane_update(&ctx, &orphaned_tabs);
                    self.scratchpad = Some(scratchpad);
                    self.execute_scratchpad_commands(commands);
                    self.save_scratchpad_state_to_disk();
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
                        .map(|t| EventTabInfo {
                            tab_id: t.tab_id,
                            position: t.position,
                            name: t.name.clone(),
                            active: t.active,
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
                        .map(|t| EventTabInfo {
                            tab_id: t.tab_id,
                            position: t.position,
                            name: String::new(),
                            active: t.active,
                        })
                        .collect();
                    self.event_stream.update_tab_state(&cheap_tabs);
                }

                // Rebuild tab ID maps
                self.tab_id_to_position.clear();
                self.position_to_tab_id.clear();
                for t in &tab_infos {
                    self.tab_id_to_position.insert(t.tab_id, t.position);
                    self.position_to_tab_id.insert(t.position, t.tab_id);
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
                        self.config_last_checked = Some(std::time::SystemTime::now());
                    }

                    // Load the config
                    let configs = self.load_merged_configs();

                    if let Some(ref mut scratchpad) = self.scratchpad {
                        let commands = scratchpad.reconcile_config(configs);
                        self.execute_scratchpad_commands(commands);
                    } else if !configs.is_empty() {
                        self.scratchpad = Some(self.new_scratchpad_manager(configs));
                    }

                    // Start polling timer if watching is enabled
                    if self.watch_interval_ms.is_some() {
                        self.schedule_timer();
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
                self.refresh_client_list();

                // Check if config file has changed (only if watching is enabled)
                if let (Some(ref include_path), Some(interval_ms)) =
                    (&self.include_path, self.watch_interval_ms)
                {
                    let now = std::time::SystemTime::now();
                    let should_check = self
                        .config_last_checked
                        .and_then(|last| now.duration_since(last).ok())
                        .map(|elapsed| elapsed.as_millis() >= u128::from(interval_ms))
                        .unwrap_or(true);

                    if should_check {
                        self.config_last_checked = Some(now);

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
                                    self.scratchpad =
                                        Some(self.new_scratchpad_manager(new_configs));
                                }
                            }
                        }
                    }
                }

                self.schedule_timer();
            }
            Event::ListClients(clients) => {
                if let Some(own_client_id) = self.own_client_id {
                    self.own_client_connected = clients
                        .iter()
                        .any(|client| client.client_id == own_client_id && client.is_current_client);
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
