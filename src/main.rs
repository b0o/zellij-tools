use std::collections::{BTreeMap, HashMap, HashSet};
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
    build_scratchpad_keybind_reconfigure, parse_scratchpad_action, parse_scratchpads_kdl,
    acquire_registry_lock, registry_file_path, registry_lock_path, registry_temp_file_path,
    OpenDecision, RegistryLockMetadata, RegistryRecordState, ScratchpadCommand, ScratchpadConfig,
    ScratchpadContext, ScratchpadKeybindUnbind, ScratchpadListQuery, ScratchpadManager,
    ScratchpadRegistry, ScratchpadAction, ScratchpadActionTarget,
};
use zellij_tools::tree;

const REGISTRY_LOCK_STALE_TIMEOUT_MS: u64 = 2_000;
const REGISTRY_PENDING_TIMEOUT_MS: u64 = 2_000;

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

    // Zellij runtime identity and permissions
    zellij_pid: Option<u32>,
    own_plugin_id: Option<u32>,
    own_client_id: Option<ClientId>,
    reconfigure_allowed: bool,

    // Last merged scratchpad config, used for client-local keybind registration.
    scratchpad_configs: HashMap<String, ScratchpadConfig>,
    installed_scratchpad_keybinds: Vec<ScratchpadKeybindUnbind>,

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
        self.build_scratchpad_context_for_tab_id(None)
    }

    fn build_scratchpad_context_for_tab_id(
        &self,
        target_tab_id: Option<usize>,
    ) -> ScratchpadContext<'_> {
        let current_tab_id = target_tab_id.or_else(|| {
            self.position_to_tab_id
                .get(&self.current_tab_position)
                .copied()
        });
        let current_tab_position = current_tab_id
            .and_then(|tab_id| self.tab_id_to_position.get(&tab_id).copied())
            .unwrap_or(self.current_tab_position);
        let (viewport_cols, viewport_rows) = self
            .tab_infos
            .iter()
            .find(|t| Some(t.tab_id) == current_tab_id)
            .or_else(|| self.tab_infos.iter().find(|t| t.active))
            .map(|t| (t.viewport_columns, t.viewport_rows))
            .unwrap_or((0, 0));

        ScratchpadContext {
            pane_manifest: &self.pane_manifest,
            current_tab_position,
            current_tab_id,
            are_floating_panes_visible: self.are_floating_panes_visible,
            tab_id_to_position: &self.tab_id_to_position,
            viewport_cols,
            viewport_rows,
        }
    }

    fn tab_id_for_source_pane(&self, pane_id: PaneId) -> Option<usize> {
        self.pane_manifest.iter().find_map(|(tab_position, panes)| {
            let found = panes.iter().any(|pane| match pane_id {
                PaneId::Terminal(id) => !pane.is_plugin && pane.id == id,
                PaneId::Plugin(id) => pane.is_plugin && pane.id == id,
            });
            found.then(|| self.position_to_tab_id.get(tab_position).copied())?
        })
    }

    fn target_tab_id(&self, target: &ScratchpadActionTarget) -> Option<usize> {
        target
            .tab_id
            .or_else(|| target.source_pane.and_then(|pane_id| self.tab_id_for_source_pane(pane_id)))
    }

    fn scratchpad_action_target_tab_id(&self, action: &ScratchpadAction) -> Option<usize> {
        match action {
            ScratchpadAction::Toggle { target, .. }
            | ScratchpadAction::Show { target, .. }
            | ScratchpadAction::Hide { target, .. }
            | ScratchpadAction::Close { target, .. } => self.target_tab_id(target),
        }
    }

    fn current_time_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
            .unwrap_or_default()
    }

    fn live_registry_state(&self) -> (HashSet<usize>, HashMap<u32, usize>) {
        let live_tabs = self.tab_infos.iter().map(|tab| tab.tab_id).collect();
        let live_panes = self
            .pane_manifest
            .iter()
            .filter_map(|(tab_position, panes)| {
                self.position_to_tab_id
                    .get(tab_position)
                    .map(|tab_id| (*tab_id, panes))
            })
            .flat_map(|(tab_id, panes)| {
                panes
                    .iter()
                    .filter(|pane| !pane.is_plugin)
                    .map(move |pane| (pane.id, tab_id))
            })
            .collect();

        (live_tabs, live_panes)
    }

    fn with_scratchpad_registry<T>(
        &self,
        f: impl FnOnce(&mut ScratchpadRegistry, u32, u64) -> T,
    ) -> Option<T> {
        let zellij_pid = self.zellij_pid?;
        let own_plugin_id = self.own_plugin_id?;
        let now_ms = Self::current_time_ms();
        let metadata = RegistryLockMetadata {
            plugin_id: own_plugin_id,
            client_id: self.own_client_id.map(|client_id| client_id as u32).unwrap_or_default(),
            created_ms: now_ms,
        };
        let lock_path = registry_lock_path(zellij_pid);
        let _lock = match acquire_registry_lock(
            &lock_path,
            &metadata,
            REGISTRY_LOCK_STALE_TIMEOUT_MS,
        ) {
            Ok(Some(lock)) => lock,
            Ok(None) => return None,
            Err(err) => {
                eprintln!("Failed to acquire scratchpad registry lock: {}", err);
                return None;
            }
        };

        let path = registry_file_path(zellij_pid);
        let temp_path = registry_temp_file_path(zellij_pid, own_plugin_id);
        let mut registry = match ScratchpadRegistry::read_from_path(&path) {
            Ok(registry) => registry,
            Err(err) => {
                eprintln!("Failed to read scratchpad registry: {}", err);
                return None;
            }
        };

        let (live_tabs, live_panes) = self.live_registry_state();
        registry.reconcile(
            &live_tabs,
            &live_panes,
            now_ms,
            REGISTRY_PENDING_TIMEOUT_MS,
        );
        let result = f(&mut registry, own_plugin_id, now_ms);

        if let Err(err) = registry.write_atomic_to_path(&path, &temp_path) {
            eprintln!("Failed to write scratchpad registry: {}", err);
            return None;
        }

        Some(result)
    }

    fn execute_register_commands(commands: Vec<ScratchpadCommand>) {
        for command in commands {
            if let ScratchpadCommand::RenamePane { pane_id, name } = command {
                rename_terminal_pane(pane_id, &name);
            }
        }
    }

    fn register_existing_scratchpad_pane(&mut self, name: &str, tab_id: usize, pane_id: u32) {
        if let Some(ref mut mgr) = self.scratchpad {
            let register_cmds = mgr.register_pane(name, tab_id, pane_id);
            Self::execute_register_commands(register_cmds);
        }
    }

    /// Refresh the scratchpad manager's pane map from the shared registry so
    /// toggle/show/hide decisions account for scratchpads opened by other
    /// clients' plugin instances. Without this, an instance that did not open a
    /// scratchpad believes no pane exists and re-shows instead of hiding it.
    fn sync_scratchpad_panes_from_registry(&mut self) {
        let known = self.with_scratchpad_registry(|registry, _owner, _now_ms| {
            registry
                .entries
                .iter()
                .filter_map(|record| match record.state {
                    RegistryRecordState::Present { pane_id } => {
                        Some((record.name.clone(), record.tab_id, pane_id))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        });

        if let (Some(known), Some(mgr)) = (known, self.scratchpad.as_mut()) {
            mgr.sync_known_panes(&known);
        }
    }

    fn register_configured_scratchpad_keybinds(&mut self) {
        if !self.reconfigure_allowed {
            return;
        }
        let Some(own_plugin_id) = self.own_plugin_id else {
            return;
        };

        let (keys_to_unbind, new_installed, keybind_config) =
            match build_scratchpad_keybind_reconfigure(
                &self.scratchpad_configs,
                own_plugin_id,
                &self.installed_scratchpad_keybinds,
            ) {
                Ok(update) => update,
                Err(err) => {
                    eprintln!("Failed to build scratchpad keybind config: {}", err);
                    return;
                }
            };

        if !keys_to_unbind.is_empty() {
            rebind_keys(keys_to_unbind, Vec::new(), false);
        }
        if !new_installed.is_empty() {
            reconfigure(keybind_config, false);
        }
        self.installed_scratchpad_keybinds = new_installed;
    }

    fn replace_scratchpad_configs(&mut self, configs: HashMap<String, ScratchpadConfig>) {
        self.scratchpad_configs = configs.clone();
        self.register_configured_scratchpad_keybinds();

        if let Some(ref mut scratchpad) = self.scratchpad {
            let commands = scratchpad.reconcile_config(configs);
            self.execute_scratchpad_commands(commands);
        } else if !configs.is_empty() {
            self.scratchpad = Some(ScratchpadManager::new(configs));
        }
    }

    fn execute_scratchpad_commands(&mut self, commands: Vec<ScratchpadCommand>) {
        for cmd in commands {
            match cmd {
                ScratchpadCommand::OpenFloating {
                    command,
                    coordinates,
                    name,
                    tab_id,
                } => {
                    let open_decision = self.with_scratchpad_registry(|registry, owner, now_ms| {
                        registry.begin_open(
                            &name,
                            tab_id,
                            owner,
                            now_ms,
                            REGISTRY_PENDING_TIMEOUT_MS,
                        )
                    });

                    match open_decision {
                        Some(OpenDecision::UseExisting { pane_id }) => {
                            self.register_existing_scratchpad_pane(&name, tab_id, pane_id);
                            show_pane_with_id(PaneId::Terminal(pane_id), true, true);
                            continue;
                        }
                        Some(OpenDecision::Pending) => continue,
                        Some(OpenDecision::Open) | None => (),
                    }

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
                        self.with_scratchpad_registry(|registry, owner, now_ms| {
                            registry.finish_open(&name, tab_id, owner, pane_id, now_ms);
                        });
                        self.register_existing_scratchpad_pane(&name, tab_id, pane_id);
                    } else {
                        self.with_scratchpad_registry(|registry, owner, _now_ms| {
                            registry.cancel_open(&name, tab_id, owner);
                        });
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
                let target_tab_id = self.scratchpad_action_target_tab_id(&action);
                // Pull shared registry state so the decision reflects scratchpads
                // opened by other clients' plugin instances (multi-client).
                self.sync_scratchpad_panes_from_registry();
                if let Some(mut scratchpad) = self.scratchpad.take() {
                    let ctx = self.build_scratchpad_context_for_tab_id(target_tab_id);
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
        self.own_plugin_id = Some(plugin_ids.plugin_id);
        self.own_client_id = Some(plugin_ids.client_id);

        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
            PermissionType::ReadCliPipes,
            PermissionType::FullHdAccess,
            PermissionType::Reconfigure,
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
                self.replace_scratchpad_configs(configs);
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
                    }

                    // Load the config
                    let configs = self.load_merged_configs();
                    self.replace_scratchpad_configs(configs);

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
                if result == PermissionStatus::Granted {
                    self.reconfigure_allowed = true;
                    self.register_configured_scratchpad_keybinds();
                }

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
                            self.replace_scratchpad_configs(new_configs);
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
