use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The type of a pane, mirroring zellij's `PaneId` enum variants.
/// Zellij uses separate ID namespaces for terminal and plugin panes,
/// so the same numeric ID can refer to both types simultaneously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneType {
    Terminal,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventKind {
    PaneFocused,
    PaneUnfocused,
    PaneOpened,
    PaneClosed,
    TabFocused,
    TabUnfocused,
    TabCreated,
    TabClosed,
    TabMoved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypedPaneId {
    Terminal(u32),
    Plugin(u32),
}

impl TypedPaneId {
    fn from_parts(prefix: &str, value: u32) -> Result<Self, String> {
        match prefix {
            "terminal" => Ok(Self::Terminal(value)),
            "plugin" => Ok(Self::Plugin(value)),
            _ => Err(format!("invalid pane type prefix: {prefix}")),
        }
    }

    fn from_event(pane_id: u32, pane_type: PaneType) -> Self {
        match pane_type {
            PaneType::Terminal => Self::Terminal(pane_id),
            PaneType::Plugin => Self::Plugin(pane_id),
        }
    }
}

impl FromStr for TypedPaneId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (prefix, id) = s
            .split_once('_')
            .ok_or_else(|| format!("invalid typed pane id: {s}"))?;
        let value = id
            .parse::<u32>()
            .map_err(|_| format!("invalid typed pane numeric id: {s}"))?;
        Self::from_parts(prefix, value)
    }
}

impl<'de> Deserialize<'de> for TypedPaneId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SubscribeSpec {
    #[serde(default)]
    pub full: Option<bool>,
    #[serde(default)]
    pub events: Option<Vec<EventKind>>,
    #[serde(default)]
    pub pane_ids: Option<Vec<TypedPaneId>>,
    #[serde(default)]
    pub tab_ids: Option<Vec<u64>>,
}

#[derive(Debug, Clone, Default)]
struct EventFilter {
    event_kinds: Option<HashSet<EventKind>>,
    pane_ids: Option<HashSet<TypedPaneId>>,
    tab_ids: Option<HashSet<u64>>,
}

impl EventFilter {
    fn from_spec(spec: &SubscribeSpec) -> Self {
        Self {
            event_kinds: spec
                .events
                .as_ref()
                .map(|kinds| kinds.iter().copied().collect()),
            pane_ids: spec
                .pane_ids
                .as_ref()
                .map(|pane_ids| pane_ids.iter().copied().collect()),
            tab_ids: spec
                .tab_ids
                .as_ref()
                .map(|tab_ids| tab_ids.iter().copied().collect()),
        }
    }

    fn matches(&self, event: &Event) -> bool {
        if let Some(event_kinds) = &self.event_kinds {
            match event.kind() {
                Some(kind) if event_kinds.contains(&kind) => {}
                _ => return false,
            }
        }

        if let Some(pane_ids) = &self.pane_ids {
            match event.pane_key() {
                Some(pane_id) if pane_ids.contains(&pane_id) => {}
                _ => return false,
            }
        }

        if let Some(tab_ids) = &self.tab_ids {
            match event.tab_stable_id() {
                Some(tab_id) if tab_ids.contains(&tab_id) => {}
                _ => return false,
            }
        }

        true
    }
}

/// An event emitted to subscribers.
///
/// Serialized as an externally-tagged JSON object, e.g.:
/// `{"PaneFocused":{"pane_id":3,"pane_type":"terminal"}}`
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum Event {
    PaneFocused {
        pane_id: u32,
        pane_type: PaneType,
    },
    PaneUnfocused {
        pane_id: u32,
        pane_type: PaneType,
    },
    PaneOpened {
        pane_id: u32,
        pane_type: PaneType,
        is_floating: bool,
    },
    PaneClosed {
        pane_id: u32,
        pane_type: PaneType,
    },
    TabFocused {
        stable_id: u64,
        position: usize,
        name: String,
    },
    TabUnfocused {
        stable_id: u64,
        position: usize,
        name: String,
    },
    TabCreated {
        stable_id: u64,
        position: usize,
        name: String,
    },
    TabClosed {
        stable_id: u64,
        position: usize,
        name: String,
    },
    TabMoved {
        stable_id: u64,
        old_position: usize,
        new_position: usize,
        name: String,
    },
    /// Acknowledgment sent to CLI after a successful subscribe.
    Ack {},
    /// Acknowledgment sent after successful init payload parsing.
    InitAck {},
    /// Error response sent when init payload is rejected.
    InitError {
        message: String,
    },
}

impl Event {
    fn kind(&self) -> Option<EventKind> {
        match self {
            Self::PaneFocused { .. } => Some(EventKind::PaneFocused),
            Self::PaneUnfocused { .. } => Some(EventKind::PaneUnfocused),
            Self::PaneOpened { .. } => Some(EventKind::PaneOpened),
            Self::PaneClosed { .. } => Some(EventKind::PaneClosed),
            Self::TabFocused { .. } => Some(EventKind::TabFocused),
            Self::TabUnfocused { .. } => Some(EventKind::TabUnfocused),
            Self::TabCreated { .. } => Some(EventKind::TabCreated),
            Self::TabClosed { .. } => Some(EventKind::TabClosed),
            Self::TabMoved { .. } => Some(EventKind::TabMoved),
            Self::Ack {} | Self::InitAck {} | Self::InitError { .. } => None,
        }
    }

    fn pane_key(&self) -> Option<TypedPaneId> {
        match self {
            Self::PaneFocused { pane_id, pane_type }
            | Self::PaneUnfocused { pane_id, pane_type }
            | Self::PaneOpened {
                pane_id, pane_type, ..
            }
            | Self::PaneClosed { pane_id, pane_type } => {
                Some(TypedPaneId::from_event(*pane_id, *pane_type))
            }
            _ => None,
        }
    }

    fn tab_stable_id(&self) -> Option<u64> {
        match self {
            Self::TabFocused { stable_id, .. }
            | Self::TabUnfocused { stable_id, .. }
            | Self::TabCreated { stable_id, .. }
            | Self::TabClosed { stable_id, .. }
            | Self::TabMoved { stable_id, .. } => Some(*stable_id),
            _ => None,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }

    /// Serialize with full detail from pane/tab context.
    /// For pane events, looks up the pane and includes all its fields.
    /// For tab events, looks up the tab and includes all its fields.
    pub fn to_full_json(&self, panes: &[PaneInfo], tabs: &[TabInfo]) -> String {
        let mut value = serde_json::to_value(self).unwrap();

        match self {
            Event::PaneFocused { pane_id, pane_type }
            | Event::PaneUnfocused { pane_id, pane_type }
            | Event::PaneClosed { pane_id, pane_type }
            | Event::PaneOpened {
                pane_id, pane_type, ..
            } => {
                let is_plugin = *pane_type == PaneType::Plugin;
                if let Some(pane) = panes
                    .iter()
                    .find(|p| p.id == *pane_id && p.is_plugin == is_plugin)
                {
                    Self::merge_pane_detail(&mut value, pane);
                }
            }
            Event::TabFocused { stable_id, .. }
            | Event::TabUnfocused { stable_id, .. }
            | Event::TabCreated { stable_id, .. }
            | Event::TabClosed { stable_id, .. } => {
                if let Some(tab) = tabs.iter().find(|t| t.stable_id == *stable_id) {
                    Self::merge_tab_detail(&mut value, tab);
                }
            }
            Event::TabMoved { stable_id, .. } => {
                if let Some(tab) = tabs.iter().find(|t| t.stable_id == *stable_id) {
                    Self::merge_tab_detail(&mut value, tab);
                }
            }
            Event::Ack {} | Event::InitAck {} | Event::InitError { .. } => {}
        }

        serde_json::to_string(&value).unwrap()
    }

    /// Merge pane detail fields into the event's inner object
    fn merge_pane_detail(value: &mut serde_json::Value, pane: &PaneInfo) {
        // The event is {"EventName": {fields}} — get the inner object
        if let Some(obj) = value.as_object_mut() {
            for (_, inner) in obj.iter_mut() {
                if let Some(inner_obj) = inner.as_object_mut() {
                    inner_obj.insert(
                        "title".to_string(),
                        serde_json::Value::String(pane.title.clone()),
                    );
                    inner_obj.insert(
                        "is_floating".to_string(),
                        serde_json::Value::Bool(pane.is_floating),
                    );
                    inner_obj.insert(
                        "is_suppressed".to_string(),
                        serde_json::Value::Bool(pane.is_suppressed),
                    );
                    if let Some(ref cmd) = pane.terminal_command {
                        inner_obj.insert(
                            "terminal_command".to_string(),
                            serde_json::Value::String(cmd.clone()),
                        );
                    }
                    if let Some(ref url) = pane.plugin_url {
                        inner_obj.insert(
                            "plugin_url".to_string(),
                            serde_json::Value::String(url.clone()),
                        );
                    }
                }
            }
        }
    }

    /// Merge tab detail fields into the event's inner object
    fn merge_tab_detail(value: &mut serde_json::Value, _tab: &TabInfo) {
        // TabInfo already has all fields in the event (stable_id, position, name)
        // Nothing extra to merge for now
        let _ = value;
    }
}

/// Subscriber mode: controls how much detail is included in events
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscribeMode {
    /// Minimal events with just IDs
    Compact,
    /// Full events with all object fields
    Full,
}

/// A subscriber to the event stream
#[derive(Debug, Clone)]
struct Subscriber {
    /// The CLI pipe ID to send events to
    pipe_id: String,
    /// Whether to include full object details in events
    mode: SubscribeMode,
    /// Monotonic counter value at last heartbeat from this subscriber.
    last_heartbeat: u64,
    state: SubscriberState,
}

#[derive(Debug, Clone)]
enum SubscriberState {
    PendingInit,
    Active { filter: EventFilter },
}

#[derive(Debug, Clone, Serialize)]
pub enum InitError {
    InvalidJson { message: String },
    SubscriberNotFound { pipe_id: String },
    SubscriberAlreadyActive { pipe_id: String },
}

impl InitError {
    pub fn message(&self) -> String {
        match self {
            Self::InvalidJson { message } => format!("invalid init json: {message}"),
            Self::SubscriberNotFound { pipe_id } => {
                format!("subscriber not found: {pipe_id}")
            }
            Self::SubscriberAlreadyActive { pipe_id } => {
                format!("subscriber already active: {pipe_id}")
            }
        }
    }
}

impl Subscriber {
    fn pending(pipe_id: String, mode: SubscribeMode, heartbeat_counter: u64) -> Self {
        Self {
            pipe_id,
            mode,
            last_heartbeat: heartbeat_counter,
            state: SubscriberState::PendingInit,
        }
    }

    fn active(
        pipe_id: String,
        mode: SubscribeMode,
        heartbeat_counter: u64,
        filter: EventFilter,
    ) -> Self {
        Self {
            pipe_id,
            mode,
            last_heartbeat: heartbeat_counter,
            state: SubscriberState::Active { filter },
        }
    }

    fn is_active(&self) -> bool {
        matches!(self.state, SubscriberState::Active { .. })
    }
}

/// A unique identifier for a pane, combining the numeric ID with the pane type.
/// Zellij uses separate ID namespaces for terminal and plugin panes, so the same numeric ID
/// can refer to both a terminal pane and a plugin pane simultaneously.
type PaneKey = (u32, PaneType);

/// Lightweight snapshot of a pane's focus-relevant fields.
/// Stored in EventStream so that tab switches can recompute pane focus
/// without needing a new PaneUpdate. No heap allocations.
#[derive(Debug, Clone)]
struct PaneFocusSnapshot {
    id: u32,
    pane_type: PaneType,
    is_focused: bool,
    is_floating: bool,
    is_suppressed: bool,
    tab_position: usize,
}

/// Tracks state and emits events to subscribers when things change.
///
/// Detects:
/// - Pane focus changes (by comparing focused pane across PaneUpdate calls)
/// - Pane open/close (by diffing the set of known pane IDs across PaneUpdate calls)
/// - Tab focus changes (by comparing active tab across TabUpdate calls)
/// - Tab create/close (by diffing the set of known tab positions across TabUpdate calls)
///
/// Pane focus is recomputed on both PaneUpdate and TabUpdate, because zellij
/// delivers these events independently in arbitrary order. A tab switch must
/// trigger pane focus recomputation even if no PaneUpdate has arrived yet.
#[derive(Debug, Default)]
pub struct EventStream {
    /// Active subscribers keyed by pipe ID
    subscribers: HashMap<String, Subscriber>,
    /// Monotonic counter incremented on each heartbeat (used instead of time in WASI)
    heartbeat_counter: u64,
    /// Last known focused pane (pane_id, pane_type)
    last_focused_pane: Option<(u32, PaneType)>,
    /// Set of pane keys we knew about last time.
    /// Uses (id, is_plugin) to distinguish terminal from plugin panes,
    /// since zellij allows overlapping numeric IDs between the two namespaces.
    known_panes: HashSet<PaneKey>,
    /// Last known active tab (by stable_id)
    last_active_tab: Option<u64>,
    /// Known tabs by stable_id: stable_id -> (position, name)
    known_tabs: HashMap<u64, (usize, String)>,
    /// Lightweight snapshot of pane focus state for recomputing focus on tab switch.
    /// Updated on each PaneUpdate.
    pane_focus_snapshot: Vec<PaneFocusSnapshot>,
    /// Last known active tab position (by tab index, not stable_id).
    /// Used for pane focus filtering — we need the position, not stable_id,
    /// because PaneInfo.tab_position uses position.
    active_tab_position: Option<usize>,
}

/// Information about a tab, used for diffing
pub struct TabInfo {
    pub stable_id: u64,
    pub position: usize,
    pub name: String,
    pub active: bool,
}

/// Information about a pane from the manifest, used for diffing and detail
pub struct PaneInfo {
    pub id: u32,
    pub is_focused: bool,
    pub is_floating: bool,
    pub is_suppressed: bool,
    pub is_plugin: bool,
    /// Which tab this pane belongs to (tab position index)
    pub tab_position: usize,
    // Extra fields for full mode
    pub title: String,
    pub terminal_command: Option<String>,
    pub plugin_url: Option<String>,
}

impl PaneInfo {
    /// Get the pane type (terminal or plugin) for this pane.
    pub fn pane_type(&self) -> PaneType {
        if self.is_plugin {
            PaneType::Plugin
        } else {
            PaneType::Terminal
        }
    }
}

impl EventStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a subscriber. Returns a vec of events representing current state
    /// (so the subscriber gets an initial snapshot).
    pub fn subscribe(&mut self, pipe_id: String, mode: SubscribeMode) -> Vec<Event> {
        let subscriber = Subscriber::active(
            pipe_id.clone(),
            mode,
            self.heartbeat_counter,
            EventFilter::default(),
        );
        self.subscribers.insert(pipe_id, subscriber);

        let mut initial_events = Vec::new();

        // Send current tab focus state
        if let Some(stable_id) = self.last_active_tab {
            if let Some((position, name)) = self.known_tabs.get(&stable_id) {
                initial_events.push(Event::TabFocused {
                    stable_id,
                    position: *position,
                    name: name.clone(),
                });
            }
        }

        // Send current pane focus state
        if let Some((pane_id, pane_type)) = self.last_focused_pane {
            initial_events.push(Event::PaneFocused { pane_id, pane_type });
        }

        initial_events
    }

    /// Add a subscriber that must be initialized before receiving stream events.
    pub fn subscribe_pending(&mut self, pipe_id: String, mode: SubscribeMode) {
        let subscriber = Subscriber::pending(pipe_id.clone(), mode, self.heartbeat_counter);
        self.subscribers.insert(pipe_id, subscriber);
    }

    /// Initialize a pending subscriber from an init JSON payload.
    pub fn initialize_subscriber(
        &mut self,
        pipe_id: &str,
        spec_json: &str,
    ) -> Result<(), InitError> {
        let spec = serde_json::from_str::<SubscribeSpec>(spec_json).map_err(|err| {
            InitError::InvalidJson {
                message: err.to_string(),
            }
        })?;
        let sub =
            self.subscribers
                .get_mut(pipe_id)
                .ok_or_else(|| InitError::SubscriberNotFound {
                    pipe_id: pipe_id.to_string(),
                })?;

        if sub.is_active() {
            return Err(InitError::SubscriberAlreadyActive {
                pipe_id: pipe_id.to_string(),
            });
        }

        if let Some(full) = spec.full {
            sub.mode = if full {
                SubscribeMode::Full
            } else {
                SubscribeMode::Compact
            };
        }
        sub.state = SubscriberState::Active {
            filter: EventFilter::from_spec(&spec),
        };
        Ok(())
    }

    pub fn is_active(&self, pipe_id: &str) -> bool {
        self.subscribers
            .get(pipe_id)
            .map(Subscriber::is_active)
            .unwrap_or(false)
    }

    pub fn is_pending(&self, pipe_id: &str) -> bool {
        self.subscribers
            .get(pipe_id)
            .map(|sub| matches!(sub.state, SubscriberState::PendingInit))
            .unwrap_or(false)
    }

    /// Record a heartbeat from a subscriber (called on empty pipe messages).
    pub fn record_heartbeat(&mut self, pipe_id: &str) {
        self.heartbeat_counter += 1;
        if let Some(sub) = self.subscribers.get_mut(pipe_id) {
            sub.last_heartbeat = self.heartbeat_counter;
        }
    }

    /// Prune subscribers that haven't sent a heartbeat in `max_missed` ticks.
    /// Returns the pipe IDs of pruned subscribers.
    pub fn prune_stale_subscribers(&mut self, max_missed: u64) -> Vec<String> {
        let threshold = self.heartbeat_counter.saturating_sub(max_missed);
        let stale: Vec<String> = self
            .subscribers
            .iter()
            .filter(|(_, sub)| sub.last_heartbeat < threshold)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            self.subscribers.remove(id);
        }
        stale
    }

    /// Get the current heartbeat counter value.
    pub fn heartbeat_counter(&self) -> u64 {
        self.heartbeat_counter
    }

    /// Remove a subscriber
    pub fn unsubscribe(&mut self, pipe_id: &str) {
        self.subscribers.remove(pipe_id);
    }

    /// Check if there are any subscribers
    pub fn has_subscribers(&self) -> bool {
        !self.subscribers.is_empty()
    }

    /// Process a pane update, returns events to broadcast to all subscribers.
    ///
    /// This method:
    /// 1. Diffs the pane set to detect opens/closes
    /// 2. Diffs focus state to detect focus changes (only considers panes on `active_tab`)
    ///
    /// Returns a vec of (pipe_id, json_event) tuples to emit.
    pub fn on_pane_update(
        &mut self,
        panes: &[PaneInfo],
        active_tab: usize,
    ) -> Vec<(String, String)> {
        if !self.has_subscribers() {
            self.update_pane_state(panes, active_tab);
            return vec![];
        }

        let events = self.compute_events(panes, active_tab);
        self.update_pane_state(panes, active_tab);

        self.broadcast_events(&events, panes, &[])
    }

    /// Process a tab update, returns events to broadcast to all subscribers.
    ///
    /// Also recomputes pane focus if the active tab position changed, because
    /// PaneUpdate and TabUpdate arrive in arbitrary order from zellij. A tab
    /// switch must trigger pane focus recomputation even without a new PaneUpdate.
    pub fn on_tab_update(&mut self, tabs: &[TabInfo]) -> Vec<(String, String)> {
        let new_active_position = tabs.iter().find(|t| t.active).map(|t| t.position);

        if !self.has_subscribers() {
            // Still recompute pane focus so last_focused_pane stays correct
            if new_active_position != self.active_tab_position {
                if let Some(pos) = new_active_position {
                    self.active_tab_position = Some(pos);
                    self.last_focused_pane =
                        Self::find_focused_from_snapshot(&self.pane_focus_snapshot, pos);
                }
            }
            self.update_tab_state(tabs);
            return vec![];
        }

        let mut events = self.compute_tab_events(tabs);

        // If active tab position changed, recompute pane focus from stored snapshot
        if new_active_position != self.active_tab_position {
            if let Some(new_pos) = new_active_position {
                let new_focused =
                    Self::find_focused_from_snapshot(&self.pane_focus_snapshot, new_pos);
                let focus_events = Self::compute_focus_change(self.last_focused_pane, new_focused);
                events.extend(focus_events);
                self.active_tab_position = Some(new_pos);
                self.last_focused_pane = new_focused;
            }
        }

        self.update_tab_state(tabs);

        self.broadcast_events(&events, &[], tabs)
    }

    /// Broadcast events to all subscribers, respecting their mode.
    /// Full-mode subscribers get extra detail fields in the JSON.
    fn broadcast_events(
        &self,
        events: &[Event],
        panes: &[PaneInfo],
        tabs: &[TabInfo],
    ) -> Vec<(String, String)> {
        let active_subscribers: Vec<&Subscriber> = self
            .subscribers
            .values()
            .filter(|s| s.is_active())
            .collect();
        if active_subscribers.is_empty() {
            return Vec::new();
        }

        let mut output = Vec::new();
        for event in events {
            let compact_json = event.to_json();
            // Only compute full JSON if any subscriber wants it
            let full_json = if active_subscribers
                .iter()
                .any(|s| s.mode == SubscribeMode::Full)
            {
                Some(event.to_full_json(panes, tabs))
            } else {
                None
            };

            for sub in &active_subscribers {
                if let SubscriberState::Active { filter } = &sub.state {
                    if !filter.matches(event) {
                        continue;
                    }
                }
                let json = match sub.mode {
                    SubscribeMode::Compact => compact_json.clone(),
                    SubscribeMode::Full => full_json.as_ref().unwrap_or(&compact_json).clone(),
                };
                output.push((sub.pipe_id.clone(), json));
            }
        }
        output
    }

    /// Compute tab events by diffing old vs new tab state.
    /// Tracks tabs by stable_id to detect moves vs create/close.
    fn compute_tab_events(&self, tabs: &[TabInfo]) -> Vec<Event> {
        let mut events = Vec::new();

        let new_ids: HashSet<u64> = tabs.iter().map(|t| t.stable_id).collect();

        // Created tabs (new stable_id not in old)
        for tab in tabs {
            if !self.known_tabs.contains_key(&tab.stable_id) {
                events.push(Event::TabCreated {
                    stable_id: tab.stable_id,
                    position: tab.position,
                    name: tab.name.clone(),
                });
            }
        }

        // Closed tabs (old stable_id not in new)
        for (&old_id, (position, name)) in &self.known_tabs {
            if !new_ids.contains(&old_id) {
                events.push(Event::TabClosed {
                    stable_id: old_id,
                    position: *position,
                    name: name.clone(),
                });
            }
        }

        // Moved tabs (same stable_id, different position)
        for tab in tabs {
            if let Some((old_position, _)) = self.known_tabs.get(&tab.stable_id) {
                if *old_position != tab.position {
                    events.push(Event::TabMoved {
                        stable_id: tab.stable_id,
                        old_position: *old_position,
                        new_position: tab.position,
                        name: tab.name.clone(),
                    });
                }
            }
        }

        // Focus changes (by stable_id, not position)
        let new_active = tabs.iter().find(|t| t.active).map(|t| t.stable_id);

        match (self.last_active_tab, new_active) {
            (Some(old_id), Some(new_id)) if old_id != new_id => {
                let (old_pos, old_name) = self.known_tabs.get(&old_id).cloned().unwrap_or_default();
                events.push(Event::TabUnfocused {
                    stable_id: old_id,
                    position: old_pos,
                    name: old_name,
                });
                if let Some(tab) = tabs.iter().find(|t| t.stable_id == new_id) {
                    events.push(Event::TabFocused {
                        stable_id: new_id,
                        position: tab.position,
                        name: tab.name.clone(),
                    });
                }
            }
            (Some(old_id), None) => {
                let (old_pos, old_name) = self.known_tabs.get(&old_id).cloned().unwrap_or_default();
                events.push(Event::TabUnfocused {
                    stable_id: old_id,
                    position: old_pos,
                    name: old_name,
                });
            }
            (None, Some(new_id)) => {
                if let Some(tab) = tabs.iter().find(|t| t.stable_id == new_id) {
                    events.push(Event::TabFocused {
                        stable_id: new_id,
                        position: tab.position,
                        name: tab.name.clone(),
                    });
                }
            }
            _ => {}
        }

        events
    }

    /// Update internal tab state
    pub fn update_tab_state(&mut self, tabs: &[TabInfo]) {
        self.known_tabs.clear();
        for t in tabs {
            self.known_tabs
                .insert(t.stable_id, (t.position, t.name.clone()));
        }
        self.last_active_tab = tabs.iter().find(|t| t.active).map(|t| t.stable_id);
    }

    /// Compute the events that should be emitted for this pane update
    fn compute_events(&self, panes: &[PaneInfo], active_tab: usize) -> Vec<Event> {
        let mut events = Vec::new();

        // 1. Detect pane opens/closes by diffing composite keys (id, pane_type)
        let new_keys: HashSet<PaneKey> = panes.iter().map(|p| (p.id, p.pane_type())).collect();
        let old_keys = &self.known_panes;

        // Opened panes (in new but not old)
        for pane in panes {
            let key = (pane.id, pane.pane_type());
            if !old_keys.contains(&key) {
                events.push(Event::PaneOpened {
                    pane_id: pane.id,
                    pane_type: pane.pane_type(),
                    is_floating: pane.is_floating,
                });
            }
        }

        // Closed panes (in old but not new)
        for &(old_id, old_pane_type) in old_keys {
            if !new_keys.contains(&(old_id, old_pane_type)) {
                events.push(Event::PaneClosed {
                    pane_id: old_id,
                    pane_type: old_pane_type,
                });
            }
        }

        // 2. Detect focus changes (only consider panes on the active tab)
        let new_focused = Self::find_focused(panes, active_tab);
        events.extend(Self::compute_focus_change(
            self.last_focused_pane,
            new_focused,
        ));

        events
    }

    /// Update internal pane state to match current pane manifest
    pub fn update_pane_state(&mut self, panes: &[PaneInfo], active_tab: usize) {
        self.known_panes = panes.iter().map(|p| (p.id, p.pane_type())).collect();
        self.last_focused_pane = Self::find_focused(panes, active_tab);
        self.active_tab_position = Some(active_tab);
        self.pane_focus_snapshot = panes
            .iter()
            .map(|p| PaneFocusSnapshot {
                id: p.id,
                pane_type: p.pane_type(),
                is_focused: p.is_focused,
                is_floating: p.is_floating,
                is_suppressed: p.is_suppressed,
                tab_position: p.tab_position,
            })
            .collect();
    }

    /// Find the focused pane from a list of panes on the active tab.
    /// Only considers panes on `active_tab` to avoid stale focus from other tabs
    /// (e.g., a floating pane on tab 0 that retains is_focused after switching to tab 1).
    /// Among active-tab panes, floating panes take precedence. Suppressed panes are excluded.
    fn find_focused(panes: &[PaneInfo], active_tab: usize) -> Option<(u32, PaneType)> {
        let mut focused_tiled: Option<(u32, PaneType)> = None;
        let mut focused_floating: Option<(u32, PaneType)> = None;

        for pane in panes {
            if pane.tab_position == active_tab && pane.is_focused && !pane.is_suppressed {
                if pane.is_floating {
                    focused_floating = Some((pane.id, pane.pane_type()));
                } else {
                    focused_tiled = Some((pane.id, pane.pane_type()));
                }
            }
        }

        focused_floating.or(focused_tiled)
    }

    /// Like `find_focused` but works on the lightweight snapshot (no PaneInfo needed).
    fn find_focused_from_snapshot(
        snapshot: &[PaneFocusSnapshot],
        active_tab: usize,
    ) -> Option<(u32, PaneType)> {
        let mut focused_tiled: Option<(u32, PaneType)> = None;
        let mut focused_floating: Option<(u32, PaneType)> = None;

        for pane in snapshot {
            if pane.tab_position == active_tab && pane.is_focused && !pane.is_suppressed {
                if pane.is_floating {
                    focused_floating = Some((pane.id, pane.pane_type));
                } else {
                    focused_tiled = Some((pane.id, pane.pane_type));
                }
            }
        }

        focused_floating.or(focused_tiled)
    }

    /// Compute pane focus change events given old and new focused pane.
    fn compute_focus_change(
        old: Option<(u32, PaneType)>,
        new: Option<(u32, PaneType)>,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        match (old, new) {
            (Some((old_id, old_type)), Some((new_id, new_type)))
                if old_id != new_id || old_type != new_type =>
            {
                events.push(Event::PaneUnfocused {
                    pane_id: old_id,
                    pane_type: old_type,
                });
                events.push(Event::PaneFocused {
                    pane_id: new_id,
                    pane_type: new_type,
                });
            }
            (Some((old_id, old_type)), None) => {
                events.push(Event::PaneUnfocused {
                    pane_id: old_id,
                    pane_type: old_type,
                });
            }
            (None, Some((new_id, new_type))) => {
                events.push(Event::PaneFocused {
                    pane_id: new_id,
                    pane_type: new_type,
                });
            }
            _ => {}
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pane(id: u32, is_focused: bool, is_floating: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_focused,
            is_floating,
            is_suppressed: false,
            is_plugin: false,
            tab_position: 0,
            title: String::new(),
            terminal_command: None,
            plugin_url: None,
        }
    }

    fn make_suppressed_pane(id: u32, is_focused: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_focused,
            is_floating: true,
            is_suppressed: true,
            is_plugin: false,
            tab_position: 0,
            title: String::new(),
            terminal_command: None,
            plugin_url: None,
        }
    }

    // --- Filter matching ---

    #[test]
    fn filter_event_kind_matches_canonical_names() {
        let filter = EventFilter::from_spec(&SubscribeSpec {
            events: Some(vec![EventKind::PaneFocused, EventKind::TabMoved]),
            pane_ids: None,
            tab_ids: None,
            full: None,
        });

        assert!(filter.matches(&Event::PaneFocused {
            pane_id: 2,
            pane_type: PaneType::Terminal,
        }));
        assert!(filter.matches(&Event::TabMoved {
            stable_id: 42,
            old_position: 0,
            new_position: 1,
            name: "dev".to_string(),
        }));
        assert!(!filter.matches(&Event::PaneClosed {
            pane_id: 2,
            pane_type: PaneType::Terminal,
        }));
    }

    #[test]
    fn filter_typed_pane_ids_distinguishes_terminal_and_plugin() {
        let filter = EventFilter::from_spec(&SubscribeSpec {
            events: None,
            pane_ids: Some(vec![
                "terminal_2".parse().unwrap(),
                "plugin_2".parse().unwrap(),
            ]),
            tab_ids: None,
            full: None,
        });

        assert!(filter.matches(&Event::PaneFocused {
            pane_id: 2,
            pane_type: PaneType::Terminal,
        }));
        assert!(filter.matches(&Event::PaneFocused {
            pane_id: 2,
            pane_type: PaneType::Plugin,
        }));
        assert!(!filter.matches(&Event::PaneFocused {
            pane_id: 3,
            pane_type: PaneType::Terminal,
        }));
    }

    #[test]
    fn filter_tab_stable_ids_match_tab_events() {
        let filter = EventFilter::from_spec(&SubscribeSpec {
            events: None,
            pane_ids: None,
            tab_ids: Some(vec![101]),
            full: None,
        });

        assert!(filter.matches(&Event::TabFocused {
            stable_id: 101,
            position: 0,
            name: "main".to_string(),
        }));
        assert!(!filter.matches(&Event::TabFocused {
            stable_id: 202,
            position: 1,
            name: "other".to_string(),
        }));
    }

    #[test]
    fn filter_omitted_fields_are_unconstrained() {
        let filter = EventFilter::from_spec(&SubscribeSpec {
            events: Some(vec![EventKind::PaneFocused]),
            pane_ids: None,
            tab_ids: None,
            full: None,
        });

        assert!(filter.matches(&Event::PaneFocused {
            pane_id: 999,
            pane_type: PaneType::Plugin,
        }));
    }

    #[test]
    fn filter_empty_list_matches_none_for_dimension() {
        let pane_filter = EventFilter::from_spec(&SubscribeSpec {
            events: None,
            pane_ids: Some(vec![]),
            tab_ids: None,
            full: None,
        });
        assert!(!pane_filter.matches(&Event::PaneFocused {
            pane_id: 2,
            pane_type: PaneType::Terminal,
        }));

        let kind_filter = EventFilter::from_spec(&SubscribeSpec {
            events: Some(vec![]),
            pane_ids: None,
            tab_ids: None,
            full: None,
        });
        assert!(!kind_filter.matches(&Event::PaneFocused {
            pane_id: 2,
            pane_type: PaneType::Terminal,
        }));

        let tab_filter = EventFilter::from_spec(&SubscribeSpec {
            events: None,
            pane_ids: None,
            tab_ids: Some(vec![]),
            full: None,
        });
        assert!(!tab_filter.matches(&Event::TabFocused {
            stable_id: 101,
            position: 0,
            name: "main".to_string(),
        }));
    }

    #[test]
    fn pending_subscribe_creates_pending_subscriber_only() {
        let mut stream = EventStream::new();
        stream.subscribe_pending("pipe-1".to_string(), SubscribeMode::Compact);

        assert!(stream.has_subscribers());
        assert!(!stream.is_active("pipe-1"));
    }

    #[test]
    fn pending_subscriber_receives_no_stream_events() {
        let mut stream = EventStream::new();
        stream.subscribe_pending("pipe-1".to_string(), SubscribeMode::Compact);

        let panes = vec![make_pane(42, true, false)];
        let pane_output = stream.on_pane_update(&panes, 0);
        assert!(pane_output.is_empty());

        let tabs = vec![make_tab(100, 0, "tab1", true)];
        let tab_output = stream.on_tab_update(&tabs);
        assert!(tab_output.is_empty());
    }

    #[test]
    fn pending_subscriber_activates_after_valid_init() {
        let mut stream = EventStream::new();
        stream.subscribe_pending("pipe-1".to_string(), SubscribeMode::Compact);

        stream
            .initialize_subscriber("pipe-1", r#"{"events":["PaneFocused"]}"#)
            .unwrap();

        assert!(stream.is_active("pipe-1"));

        let panes = vec![make_pane(42, true, false)];
        let output = stream.on_pane_update(&panes, 0);
        assert!(output
            .iter()
            .any(|(pipe, json)| { pipe == "pipe-1" && json.contains(r#""PaneFocused""#) }));
    }

    #[test]
    fn pending_subscriber_stays_pending_after_invalid_init() {
        let mut stream = EventStream::new();
        stream.subscribe_pending("pipe-1".to_string(), SubscribeMode::Compact);

        let err = stream.initialize_subscriber("pipe-1", "not-json");
        assert!(err.is_err());
        assert!(!stream.is_active("pipe-1"));

        let panes = vec![make_pane(42, true, false)];
        let output = stream.on_pane_update(&panes, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn subscribe_filter_no_events_before_init() {
        let mut stream = EventStream::new();
        stream.subscribe_pending("pipe-1".to_string(), SubscribeMode::Compact);

        let panes = vec![make_pane(42, true, false)];
        let output = stream.on_pane_update(&panes, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn subscribe_filter_only_matching_events_after_init() {
        let mut stream = EventStream::new();
        stream.subscribe_pending("pipe-1".to_string(), SubscribeMode::Compact);
        stream
            .initialize_subscriber(
                "pipe-1",
                r#"{"events":["PaneFocused"],"pane_ids":["terminal_42"]}"#,
            )
            .unwrap();

        let panes = vec![make_pane(42, true, false), make_pane(5, false, false)];
        let output = stream.on_pane_update(&panes, 0);

        assert!(output
            .iter()
            .any(|(pipe, json)| pipe == "pipe-1" && json.contains(r#""PaneFocused""#)));
        assert!(!output
            .iter()
            .any(|(pipe, json)| pipe == "pipe-1" && json.contains(r#""PaneOpened""#)));
    }

    #[test]
    fn subscribe_filter_full_mode_still_enriches_matching_events() {
        let mut stream = EventStream::new();
        stream.subscribe_pending("pipe-1".to_string(), SubscribeMode::Compact);
        stream
            .initialize_subscriber("pipe-1", r#"{"full":true,"events":["PaneFocused"]}"#)
            .unwrap();

        let panes = vec![make_detailed_pane(42, true, "shell", "/bin/zsh")];
        let output = stream.on_pane_update(&panes, 0);

        let focused = output
            .iter()
            .find(|(pipe, json)| pipe == "pipe-1" && json.contains(r#""PaneFocused""#))
            .map(|(_, json)| json)
            .expect("expected matching focused event");
        assert!(focused.contains("title"));
        assert!(focused.contains("terminal_command"));
    }

    #[test]
    fn subscribe_filter_multiple_subscribers_receive_independent_outputs() {
        let mut stream = EventStream::new();
        stream.subscribe_pending("pane-sub".to_string(), SubscribeMode::Compact);
        stream.subscribe_pending("tab-sub".to_string(), SubscribeMode::Compact);
        stream
            .initialize_subscriber("pane-sub", r#"{"events":["PaneFocused"]}"#)
            .unwrap();
        stream
            .initialize_subscriber("tab-sub", r#"{"events":["TabFocused"]}"#)
            .unwrap();

        let panes = vec![make_pane(42, true, false)];
        let pane_output = stream.on_pane_update(&panes, 0);
        assert!(pane_output
            .iter()
            .any(|(pipe, json)| pipe == "pane-sub" && json.contains(r#""PaneFocused""#)));
        assert!(!pane_output.iter().any(|(pipe, _)| pipe == "tab-sub"));

        let tabs = vec![make_tab(100, 0, "main", true)];
        let tab_output = stream.on_tab_update(&tabs);
        assert!(tab_output
            .iter()
            .any(|(pipe, json)| pipe == "tab-sub" && json.contains(r#""TabFocused""#)));
        assert!(!tab_output.iter().any(|(pipe, _)| pipe == "pane-sub"));
    }

    // --- Event serialization ---

    #[test]
    fn event_pane_focused_serializes() {
        let event = Event::PaneFocused {
            pane_id: 42,
            pane_type: PaneType::Terminal,
        };
        assert_eq!(
            event.to_json(),
            r#"{"PaneFocused":{"pane_id":42,"pane_type":"terminal"}}"#
        );
    }

    #[test]
    fn event_pane_unfocused_serializes() {
        let event = Event::PaneUnfocused {
            pane_id: 42,
            pane_type: PaneType::Terminal,
        };
        assert_eq!(
            event.to_json(),
            r#"{"PaneUnfocused":{"pane_id":42,"pane_type":"terminal"}}"#
        );
    }

    #[test]
    fn event_pane_opened_serializes() {
        let event = Event::PaneOpened {
            pane_id: 5,
            pane_type: PaneType::Terminal,
            is_floating: true,
        };
        assert_eq!(
            event.to_json(),
            r#"{"PaneOpened":{"pane_id":5,"pane_type":"terminal","is_floating":true}}"#
        );
    }

    #[test]
    fn event_pane_closed_serializes() {
        let event = Event::PaneClosed {
            pane_id: 7,
            pane_type: PaneType::Terminal,
        };
        assert_eq!(
            event.to_json(),
            r#"{"PaneClosed":{"pane_id":7,"pane_type":"terminal"}}"#
        );
    }

    // --- Subscriber management ---

    #[test]
    fn subscribe_with_no_prior_state_returns_empty() {
        let mut stream = EventStream::new();
        let events = stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        assert!(events.is_empty());
    }

    #[test]
    fn subscribe_returns_current_focus() {
        let mut stream = EventStream::new();
        // Set up state with a focused pane
        let panes = vec![make_pane(42, true, false)];
        stream.on_pane_update(&panes, 0);

        let events = stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            Event::PaneFocused {
                pane_id: 42,
                pane_type: PaneType::Terminal,
            }
        );
    }

    #[test]
    fn unsubscribe_removes_subscriber() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        assert!(stream.has_subscribers());
        stream.unsubscribe("pipe-1");
        assert!(!stream.has_subscribers());
    }

    // --- Focus change detection ---

    #[test]
    fn focus_change_emits_focused_event() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        let panes = vec![make_pane(42, true, false)];
        let output = stream.on_pane_update(&panes, 0);

        // Should have PaneOpened + PaneFocused
        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":42,"pane_type":"terminal"}}"#));
    }

    #[test]
    fn focus_switch_emits_unfocus_then_focus() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: pane 42 focused
        let panes = vec![make_pane(42, true, false), make_pane(17, false, false)];
        stream.on_pane_update(&panes, 0);

        // Switch focus to pane 17
        let panes = vec![make_pane(42, false, false), make_pane(17, true, false)];
        let output = stream.on_pane_update(&panes, 0);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneUnfocused":{"pane_id":42,"pane_type":"terminal"}}"#));
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":17,"pane_type":"terminal"}}"#));
    }

    #[test]
    fn same_focus_emits_no_focus_events() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        let panes = vec![make_pane(42, true, false)];
        stream.on_pane_update(&panes, 0);

        // Same state again
        let output = stream.on_pane_update(&panes, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn no_events_without_subscribers() {
        let mut stream = EventStream::new();
        let panes = vec![make_pane(42, true, false)];
        let output = stream.on_pane_update(&panes, 0);
        assert!(output.is_empty());
    }

    // --- Pane open/close detection ---

    #[test]
    fn new_pane_emits_opened_event() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: one pane
        let panes = vec![make_pane(1, true, false)];
        stream.on_pane_update(&panes, 0);

        // New pane appears
        let panes = vec![make_pane(1, true, false), make_pane(2, false, true)];
        let output = stream.on_pane_update(&panes, 0);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(
            &r#"{"PaneOpened":{"pane_id":2,"pane_type":"terminal","is_floating":true}}"#
        ));
    }

    #[test]
    fn removed_pane_emits_closed_event() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: two panes
        let panes = vec![make_pane(1, true, false), make_pane(2, false, false)];
        stream.on_pane_update(&panes, 0);

        // Pane 2 disappears
        let panes = vec![make_pane(1, true, false)];
        let output = stream.on_pane_update(&panes, 0);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneClosed":{"pane_id":2,"pane_type":"terminal"}}"#));
    }

    // --- Overlapping pane ID detection ---

    fn make_plugin_pane(id: u32, is_focused: bool, is_suppressed: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_focused,
            is_floating: false,
            is_suppressed,
            is_plugin: true,
            tab_position: 0,
            title: String::new(),
            terminal_command: None,
            plugin_url: Some("plugin://test".to_string()),
        }
    }

    #[test]
    fn terminal_pane_open_detected_despite_same_id_plugin_pane() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: one terminal pane (0) and suppressed plugin panes (0, 1, 2)
        // This matches real zellij behavior where plugin and terminal IDs overlap
        let panes = vec![
            make_pane(0, true, false),
            make_plugin_pane(0, false, true),
            make_plugin_pane(1, false, true),
            make_plugin_pane(2, false, true),
        ];
        stream.on_pane_update(&panes, 0);

        // User opens a new terminal pane — terminal pane 1 has same ID as plugin pane 1
        let panes = vec![
            make_pane(0, false, false),
            make_pane(1, true, false), // new terminal pane
            make_plugin_pane(0, false, true),
            make_plugin_pane(1, false, true),
            make_plugin_pane(2, false, true),
        ];
        let output = stream.on_pane_update(&panes, 0);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(
            jsons.contains(
                &r#"{"PaneOpened":{"pane_id":1,"pane_type":"terminal","is_floating":false}}"#
            ),
            "Should detect terminal pane 1 opening despite plugin pane 1 existing. Got: {:?}",
            jsons
        );
    }

    #[test]
    fn terminal_pane_close_detected_despite_same_id_plugin_pane() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: two terminal panes (0, 1) and suppressed plugin panes (0, 1, 2)
        let panes = vec![
            make_pane(0, false, false),
            make_pane(1, true, false),
            make_plugin_pane(0, false, true),
            make_plugin_pane(1, false, true),
            make_plugin_pane(2, false, true),
        ];
        stream.on_pane_update(&panes, 0);

        // User closes terminal pane 1 — plugin pane 1 still exists
        let panes = vec![
            make_pane(0, true, false),
            make_plugin_pane(0, false, true),
            make_plugin_pane(1, false, true),
            make_plugin_pane(2, false, true),
        ];
        let output = stream.on_pane_update(&panes, 0);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(
            jsons.contains(&r#"{"PaneClosed":{"pane_id":1,"pane_type":"terminal"}}"#),
            "Should detect terminal pane 1 closing despite plugin pane 1 still existing. Got: {:?}",
            jsons
        );
    }

    // --- Floating pane focus precedence ---

    #[test]
    fn floating_pane_takes_focus_precedence() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Both tiled and floating report focused, floating wins
        let panes = vec![make_pane(1, true, false), make_pane(2, true, true)];
        let output = stream.on_pane_update(&panes, 0);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":2,"pane_type":"terminal"}}"#));
        // Pane 1 should NOT get a focused event
        assert!(!jsons.contains(&r#"{"PaneFocused":{"pane_id":1,"pane_type":"terminal"}}"#));
    }

    #[test]
    fn suppressed_pane_not_considered_focused() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        let panes = vec![make_pane(1, true, false), make_suppressed_pane(2, true)];
        let output = stream.on_pane_update(&panes, 0);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        // Pane 1 (tiled) should be focused since pane 2 is suppressed
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":1,"pane_type":"terminal"}}"#));
    }

    // --- Tab event serialization ---

    #[test]
    fn event_tab_focused_serializes() {
        let event = Event::TabFocused {
            stable_id: 100,
            position: 0,
            name: "tab1".to_string(),
        };
        assert_eq!(
            event.to_json(),
            r#"{"TabFocused":{"stable_id":100,"position":0,"name":"tab1"}}"#
        );
    }

    #[test]
    fn event_tab_created_serializes() {
        let event = Event::TabCreated {
            stable_id: 200,
            position: 2,
            name: "new-tab".to_string(),
        };
        assert_eq!(
            event.to_json(),
            r#"{"TabCreated":{"stable_id":200,"position":2,"name":"new-tab"}}"#
        );
    }

    #[test]
    fn event_tab_closed_serializes() {
        let event = Event::TabClosed {
            stable_id: 300,
            position: 1,
            name: "old-tab".to_string(),
        };
        assert_eq!(
            event.to_json(),
            r#"{"TabClosed":{"stable_id":300,"position":1,"name":"old-tab"}}"#
        );
    }

    #[test]
    fn event_tab_moved_serializes() {
        let event = Event::TabMoved {
            stable_id: 100,
            old_position: 0,
            new_position: 1,
            name: "tab1".to_string(),
        };
        assert_eq!(
            event.to_json(),
            r#"{"TabMoved":{"stable_id":100,"old_position":0,"new_position":1,"name":"tab1"}}"#
        );
    }

    // --- Tab focus detection ---

    fn make_tab(stable_id: u64, position: usize, name: &str, active: bool) -> TabInfo {
        TabInfo {
            stable_id,
            position,
            name: name.to_string(),
            active,
        }
    }

    #[test]
    fn tab_focus_change_emits_events() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: tab 0 active
        let tabs = vec![
            make_tab(100, 0, "tab1", true),
            make_tab(101, 1, "tab2", false),
        ];
        stream.on_tab_update(&tabs);

        // Switch to tab 1
        let tabs = vec![
            make_tab(100, 0, "tab1", false),
            make_tab(101, 1, "tab2", true),
        ];
        let output = stream.on_tab_update(&tabs);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"TabUnfocused":{"stable_id":100,"position":0,"name":"tab1"}}"#));
        assert!(jsons.contains(&r#"{"TabFocused":{"stable_id":101,"position":1,"name":"tab2"}}"#));
    }

    #[test]
    fn same_tab_focus_emits_nothing() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        let tabs = vec![make_tab(100, 0, "tab1", true)];
        stream.on_tab_update(&tabs);

        let output = stream.on_tab_update(&tabs);
        assert!(output.is_empty());
    }

    // --- Tab create/close detection ---

    #[test]
    fn new_tab_emits_created_event() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        let tabs = vec![make_tab(100, 0, "tab1", true)];
        stream.on_tab_update(&tabs);

        // New tab appears
        let tabs = vec![
            make_tab(100, 0, "tab1", true),
            make_tab(101, 1, "tab2", false),
        ];
        let output = stream.on_tab_update(&tabs);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"TabCreated":{"stable_id":101,"position":1,"name":"tab2"}}"#));
    }

    #[test]
    fn removed_tab_emits_closed_event() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        let tabs = vec![
            make_tab(100, 0, "tab1", true),
            make_tab(101, 1, "tab2", false),
        ];
        stream.on_tab_update(&tabs);

        // Tab 101 disappears
        let tabs = vec![make_tab(100, 0, "tab1", true)];
        let output = stream.on_tab_update(&tabs);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"TabClosed":{"stable_id":101,"position":1,"name":"tab2"}}"#));
    }

    #[test]
    fn subscribe_returns_current_active_tab() {
        let mut stream = EventStream::new();

        // Set up state with active tab
        let tabs = vec![make_tab(100, 0, "tab1", true)];
        stream.on_tab_update(&tabs);

        let events = stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        assert!(events.contains(&Event::TabFocused {
            stable_id: 100,
            position: 0,
            name: "tab1".to_string(),
        }));
    }

    // --- Tab move detection ---

    #[test]
    fn tab_swap_emits_moved_not_focus_change() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: tab A at pos 0 (active), tab B at pos 1
        let tabs = vec![
            make_tab(100, 0, "tabA", true),
            make_tab(101, 1, "tabB", false),
        ];
        stream.on_tab_update(&tabs);

        // Swap positions: tab A moves to pos 1, tab B to pos 0
        // Tab A stays active
        let tabs = vec![
            make_tab(101, 0, "tabB", false),
            make_tab(100, 1, "tabA", true),
        ];
        let output = stream.on_tab_update(&tabs);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();

        // Should see TabMoved for both, NOT TabCreated/TabClosed
        assert!(jsons.iter().any(|j| j.contains("TabMoved")));
        assert!(!jsons.iter().any(|j| j.contains("TabCreated")));
        assert!(!jsons.iter().any(|j| j.contains("TabClosed")));

        // Should NOT see focus change since tab A (100) is still active
        assert!(!jsons.iter().any(|j| j.contains("TabFocused")));
        assert!(!jsons.iter().any(|j| j.contains("TabUnfocused")));
    }

    #[test]
    fn tab_move_with_focus_change() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: tab A at pos 0 (active), tab B at pos 1
        let tabs = vec![
            make_tab(100, 0, "tabA", true),
            make_tab(101, 1, "tabB", false),
        ];
        stream.on_tab_update(&tabs);

        // Swap and focus switches to tab B
        let tabs = vec![
            make_tab(101, 0, "tabB", true),
            make_tab(100, 1, "tabA", false),
        ];
        let output = stream.on_tab_update(&tabs);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();

        // Should see moves
        assert!(jsons.iter().any(|j| j.contains("TabMoved")));
        // Should see focus change
        assert!(jsons.iter().any(|j| j.contains("TabFocused")));
        assert!(jsons.iter().any(|j| j.contains("TabUnfocused")));
    }

    // --- Multiple subscribers ---

    #[test]
    fn events_broadcast_to_all_subscribers() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        stream.subscribe("pipe-2".to_string(), SubscribeMode::Compact);

        let panes = vec![make_pane(42, true, false)];
        let output = stream.on_pane_update(&panes, 0);

        // Each event should be sent to both subscribers
        let pipe_ids: Vec<&str> = output
            .iter()
            .filter(|(_, json)| json.contains("PaneFocused"))
            .map(|(id, _)| id.as_str())
            .collect();
        assert!(pipe_ids.contains(&"pipe-1"));
        assert!(pipe_ids.contains(&"pipe-2"));
    }

    // --- Full mode ---

    fn make_detailed_pane(id: u32, is_focused: bool, title: &str, command: &str) -> PaneInfo {
        PaneInfo {
            id,
            is_focused,
            is_floating: false,
            is_suppressed: false,
            is_plugin: false,
            tab_position: 0,
            title: title.to_string(),
            terminal_command: Some(command.to_string()),
            plugin_url: None,
        }
    }

    #[test]
    fn full_mode_pane_event_includes_detail() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Full);

        let panes = vec![make_detailed_pane(42, true, "zsh", "/bin/zsh")];
        let output = stream.on_pane_update(&panes, 0);

        // Find the PaneFocused event
        let focus_json = output
            .iter()
            .find(|(_, json)| json.contains("PaneFocused"))
            .map(|(_, json)| json.clone())
            .expect("Should have PaneFocused event");

        let v: serde_json::Value = serde_json::from_str(&focus_json).unwrap();
        let inner = &v["PaneFocused"];
        assert_eq!(inner["pane_id"], 42);
        assert_eq!(inner["title"], "zsh");
        assert_eq!(inner["terminal_command"], "/bin/zsh");
        assert_eq!(inner["is_floating"], false);
    }

    #[test]
    fn compact_mode_pane_event_has_no_extra_fields() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        let panes = vec![make_detailed_pane(42, true, "zsh", "/bin/zsh")];
        let output = stream.on_pane_update(&panes, 0);

        let focus_json = output
            .iter()
            .find(|(_, json)| json.contains("PaneFocused"))
            .map(|(_, json)| json.clone())
            .expect("Should have PaneFocused event");

        // Compact mode should NOT have title
        let v: serde_json::Value = serde_json::from_str(&focus_json).unwrap();
        assert!(v["PaneFocused"]["title"].is_null());
    }

    #[test]
    fn mixed_mode_subscribers_get_different_json() {
        let mut stream = EventStream::new();
        stream.subscribe("compact".to_string(), SubscribeMode::Compact);
        stream.subscribe("full".to_string(), SubscribeMode::Full);

        let panes = vec![make_detailed_pane(42, true, "zsh", "/bin/zsh")];
        let output = stream.on_pane_update(&panes, 0);

        let compact_json = output
            .iter()
            .find(|(id, json)| id == "compact" && json.contains("PaneFocused"))
            .map(|(_, json)| json.clone())
            .expect("compact subscriber should get PaneFocused");
        let full_json = output
            .iter()
            .find(|(id, json)| id == "full" && json.contains("PaneFocused"))
            .map(|(_, json)| json.clone())
            .expect("full subscriber should get PaneFocused");

        // Full should have more data
        assert!(full_json.len() > compact_json.len());
        assert!(full_json.contains("title"));
        assert!(!compact_json.contains("title"));
    }

    #[test]
    fn event_ack_serializes() {
        let event = Event::Ack {};
        assert_eq!(event.to_json(), r#"{"Ack":{}}"#);
    }

    #[test]
    fn event_init_ack_serializes() {
        let event = Event::InitAck {};
        assert_eq!(event.to_json(), r#"{"InitAck":{}}"#);
    }

    #[test]
    fn event_init_error_serializes() {
        let event = Event::InitError {
            message: "bad spec".to_string(),
        };
        assert_eq!(event.to_json(), r#"{"InitError":{"message":"bad spec"}}"#);
    }

    // --- Heartbeat tracking ---

    #[test]
    fn record_heartbeat_updates_subscriber() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        stream.record_heartbeat("pipe-1");
        stream.record_heartbeat("pipe-1");
        assert_eq!(stream.heartbeat_counter(), 2);
        assert!(stream.has_subscribers());
    }

    #[test]
    fn prune_stale_removes_dead_subscriber() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        // Simulate 100 heartbeats from a different source (not pipe-1)
        for _ in 0..100 {
            stream.record_heartbeat("other-pipe");
        }
        let pruned = stream.prune_stale_subscribers(50);
        assert_eq!(pruned, vec!["pipe-1"]);
        assert!(!stream.has_subscribers());
    }

    #[test]
    fn prune_keeps_active_subscriber() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        // pipe-1 sends heartbeats regularly
        for _ in 0..100 {
            stream.record_heartbeat("pipe-1");
        }
        let pruned = stream.prune_stale_subscribers(50);
        assert!(pruned.is_empty());
        assert!(stream.has_subscribers());
    }

    #[test]
    fn update_pane_state_is_public() {
        let mut stream = EventStream::new();

        // Call update_pane_state directly (no subscribers needed)
        let panes = vec![make_pane(42, true, false), make_pane(17, false, false)];
        stream.update_pane_state(&panes, 0);

        // State should be tracked: subscribing now returns current focus
        let events = stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            Event::PaneFocused {
                pane_id: 42,
                pane_type: PaneType::Terminal,
            }
        );
    }

    #[test]
    fn update_tab_state_is_public() {
        let mut stream = EventStream::new();

        // Call update_tab_state directly (no subscribers needed)
        let tabs = vec![
            make_tab(100, 0, "tab1", true),
            make_tab(101, 1, "tab2", false),
        ];
        stream.update_tab_state(&tabs);

        // State should be tracked: subscribing now returns current focus
        let events = stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);
        assert!(events.contains(&Event::TabFocused {
            stable_id: 100,
            position: 0,
            name: "tab1".to_string(),
        }));
    }

    // --- Floating pane focus across tab switches ---

    #[test]
    fn floating_pane_unfocused_on_tab_switch() {
        // Bug: when a floating pane is focused and user switches tabs,
        // PaneUnfocused/PaneFocused events should fire but don't because
        // find_focused gives floating panes precedence across all tabs.
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Tab 0: tiled pane 0 (focused behind floating), floating pane 10 (focused)
        let panes = vec![
            PaneInfo {
                id: 0,
                is_focused: true,
                is_floating: false,
                is_suppressed: false,
                is_plugin: false,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
                tab_position: 0,
            },
            PaneInfo {
                id: 10,
                is_focused: true,
                is_floating: true,
                is_suppressed: false,
                is_plugin: false,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
                tab_position: 0,
            },
        ];
        stream.on_pane_update(&panes, 0);

        // Now switch to tab 1 — tiled pane 1 is focused on tab 1,
        // but floating pane 10 on tab 0 still has is_focused: true in the manifest
        let panes = vec![
            PaneInfo {
                id: 0,
                is_focused: true,
                is_floating: false,
                is_suppressed: false,
                is_plugin: false,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
                tab_position: 0,
            },
            PaneInfo {
                id: 10,
                is_focused: true,
                is_floating: true,
                is_suppressed: false,
                is_plugin: false,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
                tab_position: 0,
            },
            PaneInfo {
                id: 1,
                is_focused: true,
                is_floating: false,
                is_suppressed: false,
                is_plugin: false,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
                tab_position: 1,
            },
        ];
        let output = stream.on_pane_update(&panes, 1);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        // Should unfocus floating pane 10 and focus tiled pane 1
        assert!(
            jsons.contains(&r#"{"PaneUnfocused":{"pane_id":10,"pane_type":"terminal"}}"#),
            "Should emit PaneUnfocused for floating pane 10 when switching tabs. Got: {:?}",
            jsons
        );
        assert!(
            jsons.contains(&r#"{"PaneFocused":{"pane_id":1,"pane_type":"terminal"}}"#),
            "Should emit PaneFocused for tiled pane 1 on new tab. Got: {:?}",
            jsons
        );
    }

    #[test]
    fn tab_update_before_pane_update_emits_pane_focus_change() {
        // When TabUpdate arrives before PaneUpdate, the tab change should
        // trigger pane focus recomputation using stored pane state.
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial state: tab 0 active, floating pane 10 focused on tab 0,
        // tiled pane 1 focused on tab 1
        let tabs = vec![
            make_tab(100, 0, "tab0", true),
            make_tab(101, 1, "tab1", false),
        ];
        stream.on_tab_update(&tabs);

        let panes = vec![
            make_pane(0, true, false), // tiled pane on tab 0
            PaneInfo {
                id: 10,
                is_focused: true,
                is_floating: true,
                is_suppressed: false,
                is_plugin: false,
                tab_position: 0,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
            },
            PaneInfo {
                id: 1,
                is_focused: true,
                is_floating: false,
                is_suppressed: false,
                is_plugin: false,
                tab_position: 1,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
            },
        ];
        stream.on_pane_update(&panes, 0);

        // Now TabUpdate arrives FIRST (before any new PaneUpdate), switching to tab 1
        let tabs = vec![
            make_tab(100, 0, "tab0", false),
            make_tab(101, 1, "tab1", true),
        ];
        let output = stream.on_tab_update(&tabs);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(
            jsons.contains(&r#"{"PaneUnfocused":{"pane_id":10,"pane_type":"terminal"}}"#),
            "TabUpdate should trigger PaneUnfocused for floating pane 10. Got: {:?}",
            jsons
        );
        assert!(
            jsons.contains(&r#"{"PaneFocused":{"pane_id":1,"pane_type":"terminal"}}"#),
            "TabUpdate should trigger PaneFocused for tiled pane 1 on new tab. Got: {:?}",
            jsons
        );
    }

    #[test]
    fn pane_update_before_tab_update_emits_pane_focus_on_tab_update() {
        // When PaneUpdate arrives before TabUpdate with stale active tab,
        // the focus change should be emitted when TabUpdate arrives.
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial state: tab 0 active, floating pane 10 focused
        let tabs = vec![
            make_tab(100, 0, "tab0", true),
            make_tab(101, 1, "tab1", false),
        ];
        stream.on_tab_update(&tabs);

        let panes = vec![
            make_pane(0, true, false),
            PaneInfo {
                id: 10,
                is_focused: true,
                is_floating: true,
                is_suppressed: false,
                is_plugin: false,
                tab_position: 0,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
            },
        ];
        stream.on_pane_update(&panes, 0);

        // PaneUpdate arrives FIRST with new pane on tab 1, but active_tab still 0
        // (TabUpdate hasn't arrived yet, so caller passes old active_tab)
        let panes = vec![
            make_pane(0, true, false),
            PaneInfo {
                id: 10,
                is_focused: true,
                is_floating: true,
                is_suppressed: false,
                is_plugin: false,
                tab_position: 0,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
            },
            PaneInfo {
                id: 1,
                is_focused: true,
                is_floating: false,
                is_suppressed: false,
                is_plugin: false,
                tab_position: 1,
                title: String::new(),
                terminal_command: None,
                plugin_url: None,
            },
        ];
        let pane_output = stream.on_pane_update(&panes, 0);

        // PaneUpdate with stale active_tab should NOT emit focus change
        // (floating pane 10 is still "focused" on tab 0)
        let pane_jsons: Vec<&str> = pane_output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(
            !pane_jsons
                .iter()
                .any(|j| j.contains("PaneUnfocused") || j.contains("PaneFocused")),
            "PaneUpdate with stale active_tab should not emit focus events. Got: {:?}",
            pane_jsons
        );

        // NOW TabUpdate arrives, switching to tab 1
        let tabs = vec![
            make_tab(100, 0, "tab0", false),
            make_tab(101, 1, "tab1", true),
        ];
        let tab_output = stream.on_tab_update(&tabs);

        let tab_jsons: Vec<&str> = tab_output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(
            tab_jsons.contains(&r#"{"PaneUnfocused":{"pane_id":10,"pane_type":"terminal"}}"#),
            "TabUpdate should trigger PaneUnfocused for floating pane 10. Got: {:?}",
            tab_jsons
        );
        assert!(
            tab_jsons.contains(&r#"{"PaneFocused":{"pane_id":1,"pane_type":"terminal"}}"#),
            "TabUpdate should trigger PaneFocused for pane 1 on new tab. Got: {:?}",
            tab_jsons
        );
    }

    #[test]
    fn prune_mixed_active_and_stale() {
        let mut stream = EventStream::new();
        stream.subscribe("active".to_string(), SubscribeMode::Compact);
        stream.subscribe("stale".to_string(), SubscribeMode::Compact);
        // Only "active" sends heartbeats
        for _ in 0..100 {
            stream.record_heartbeat("active");
        }
        let pruned = stream.prune_stale_subscribers(50);
        assert_eq!(pruned, vec!["stale"]);
        assert!(stream.has_subscribers());
    }
}
