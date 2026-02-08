use std::collections::{HashMap, HashSet};

use serde::Serialize;

/// The type of a pane, mirroring zellij's `PaneId` enum variants.
/// Zellij uses separate ID namespaces for terminal and plugin panes,
/// so the same numeric ID can refer to both types simultaneously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneType {
    Terminal,
    Plugin,
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
}

impl Event {
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
pub struct Subscriber {
    /// The CLI pipe ID to send events to
    pub pipe_id: String,
    /// Whether to include full object details in events
    pub mode: SubscribeMode,
}

impl Subscriber {
    pub fn new(pipe_id: String, mode: SubscribeMode) -> Self {
        Self { pipe_id, mode }
    }
}

/// A unique identifier for a pane, combining the numeric ID with the pane type.
/// Zellij uses separate ID namespaces for terminal and plugin panes, so the same numeric ID
/// can refer to both a terminal pane and a plugin pane simultaneously.
type PaneKey = (u32, PaneType);

/// Tracks state and emits events to subscribers when things change.
///
/// Detects:
/// - Pane focus changes (by comparing focused pane across PaneUpdate calls)
/// - Pane open/close (by diffing the set of known pane IDs across PaneUpdate calls)
/// - Tab focus changes (by comparing active tab across TabUpdate calls)
/// - Tab create/close (by diffing the set of known tab positions across TabUpdate calls)
#[derive(Debug, Default)]
pub struct EventStream {
    /// Active subscribers keyed by pipe ID
    subscribers: HashMap<String, Subscriber>,
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
        let subscriber = Subscriber::new(pipe_id.clone(), mode);
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
    /// 2. Diffs focus state to detect focus changes
    ///
    /// Returns a vec of (pipe_id, json_event) tuples to emit.
    pub fn on_pane_update(&mut self, panes: &[PaneInfo]) -> Vec<(String, String)> {
        if !self.has_subscribers() {
            self.update_pane_state(panes);
            return vec![];
        }

        let events = self.compute_events(panes);
        self.update_pane_state(panes);

        self.broadcast_events(&events, panes, &[])
    }

    /// Process a tab update, returns events to broadcast to all subscribers.
    pub fn on_tab_update(&mut self, tabs: &[TabInfo]) -> Vec<(String, String)> {
        if !self.has_subscribers() {
            self.update_tab_state(tabs);
            return vec![];
        }

        let events = self.compute_tab_events(tabs);
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
        let mut output = Vec::new();
        for event in events {
            let compact_json = event.to_json();
            // Only compute full JSON if any subscriber wants it
            let full_json = if self
                .subscribers
                .values()
                .any(|s| s.mode == SubscribeMode::Full)
            {
                Some(event.to_full_json(panes, tabs))
            } else {
                None
            };

            for sub in self.subscribers.values() {
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
        let old_ids: HashSet<u64> = self.known_tabs.keys().copied().collect();

        // Created tabs (new stable_id not in old)
        for tab in tabs {
            if !old_ids.contains(&tab.stable_id) {
                events.push(Event::TabCreated {
                    stable_id: tab.stable_id,
                    position: tab.position,
                    name: tab.name.clone(),
                });
            }
        }

        // Closed tabs (old stable_id not in new)
        for &old_id in &old_ids {
            if !new_ids.contains(&old_id) {
                let (position, name) = self.known_tabs.get(&old_id).cloned().unwrap_or_default();
                events.push(Event::TabClosed {
                    stable_id: old_id,
                    position,
                    name,
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
    fn update_tab_state(&mut self, tabs: &[TabInfo]) {
        self.known_tabs = tabs
            .iter()
            .map(|t| (t.stable_id, (t.position, t.name.clone())))
            .collect();
        self.last_active_tab = tabs.iter().find(|t| t.active).map(|t| t.stable_id);
    }

    /// Compute the events that should be emitted for this pane update
    fn compute_events(&self, panes: &[PaneInfo]) -> Vec<Event> {
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

        // 2. Detect focus changes
        let new_focused = Self::find_focused(panes);

        match (self.last_focused_pane, new_focused) {
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
            _ => {} // Same pane or both None — no change
        }

        events
    }

    /// Update internal pane state to match current pane manifest
    fn update_pane_state(&mut self, panes: &[PaneInfo]) {
        self.known_panes = panes.iter().map(|p| (p.id, p.pane_type())).collect();
        self.last_focused_pane = Self::find_focused(panes);
    }

    /// Find the focused pane from a list of panes.
    /// Floating panes take precedence. Suppressed panes are excluded.
    fn find_focused(panes: &[PaneInfo]) -> Option<(u32, PaneType)> {
        let mut focused_tiled: Option<(u32, PaneType)> = None;
        let mut focused_floating: Option<(u32, PaneType)> = None;

        for pane in panes {
            if pane.is_focused && !pane.is_suppressed {
                if pane.is_floating {
                    focused_floating = Some((pane.id, pane.pane_type()));
                } else {
                    focused_tiled = Some((pane.id, pane.pane_type()));
                }
            }
        }

        focused_floating.or(focused_tiled)
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
            title: String::new(),
            terminal_command: None,
            plugin_url: None,
        }
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
        stream.on_pane_update(&panes);

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
        let output = stream.on_pane_update(&panes);

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
        stream.on_pane_update(&panes);

        // Switch focus to pane 17
        let panes = vec![make_pane(42, false, false), make_pane(17, true, false)];
        let output = stream.on_pane_update(&panes);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneUnfocused":{"pane_id":42,"pane_type":"terminal"}}"#));
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":17,"pane_type":"terminal"}}"#));
    }

    #[test]
    fn same_focus_emits_no_focus_events() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        let panes = vec![make_pane(42, true, false)];
        stream.on_pane_update(&panes);

        // Same state again
        let output = stream.on_pane_update(&panes);
        assert!(output.is_empty());
    }

    #[test]
    fn no_events_without_subscribers() {
        let mut stream = EventStream::new();
        let panes = vec![make_pane(42, true, false)];
        let output = stream.on_pane_update(&panes);
        assert!(output.is_empty());
    }

    // --- Pane open/close detection ---

    #[test]
    fn new_pane_emits_opened_event() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string(), SubscribeMode::Compact);

        // Initial: one pane
        let panes = vec![make_pane(1, true, false)];
        stream.on_pane_update(&panes);

        // New pane appears
        let panes = vec![make_pane(1, true, false), make_pane(2, false, true)];
        let output = stream.on_pane_update(&panes);

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
        stream.on_pane_update(&panes);

        // Pane 2 disappears
        let panes = vec![make_pane(1, true, false)];
        let output = stream.on_pane_update(&panes);

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
        stream.on_pane_update(&panes);

        // User opens a new terminal pane — terminal pane 1 has same ID as plugin pane 1
        let panes = vec![
            make_pane(0, false, false),
            make_pane(1, true, false), // new terminal pane
            make_plugin_pane(0, false, true),
            make_plugin_pane(1, false, true),
            make_plugin_pane(2, false, true),
        ];
        let output = stream.on_pane_update(&panes);

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
        stream.on_pane_update(&panes);

        // User closes terminal pane 1 — plugin pane 1 still exists
        let panes = vec![
            make_pane(0, true, false),
            make_plugin_pane(0, false, true),
            make_plugin_pane(1, false, true),
            make_plugin_pane(2, false, true),
        ];
        let output = stream.on_pane_update(&panes);

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
        let output = stream.on_pane_update(&panes);

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
        let output = stream.on_pane_update(&panes);

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
        let output = stream.on_pane_update(&panes);

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
        let output = stream.on_pane_update(&panes);

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
        let output = stream.on_pane_update(&panes);

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
        let output = stream.on_pane_update(&panes);

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
}
