use std::collections::{HashMap, HashSet};

use serde::Serialize;

/// An event emitted to subscribers.
///
/// Serialized as an externally-tagged JSON object, e.g.:
/// `{"pane_focused":{"pane_id":3}}`
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum Event {
    PaneFocused { pane_id: u32 },
    PaneUnfocused { pane_id: u32 },
    PaneOpened { pane_id: u32, is_floating: bool },
    PaneClosed { pane_id: u32 },
}

impl Event {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

/// A subscriber to the event stream
#[derive(Debug, Clone)]
pub struct Subscriber {
    /// The CLI pipe ID to send events to
    pub pipe_id: String,
}

impl Subscriber {
    pub fn new(pipe_id: String) -> Self {
        Self { pipe_id }
    }
}

/// Tracks state and emits events to subscribers when things change.
///
/// Detects:
/// - Pane focus changes (by comparing focused pane across PaneUpdate calls)
/// - Pane open/close (by diffing the set of known pane IDs across PaneUpdate calls)
#[derive(Debug, Default)]
pub struct EventStream {
    /// Active subscribers keyed by pipe ID
    subscribers: HashMap<String, Subscriber>,
    /// Last known focused pane (pane_id, is_plugin)
    last_focused: Option<(u32, bool)>,
    /// Set of pane IDs we knew about last time
    known_panes: HashSet<u32>,
}

/// Information about a pane from the manifest, used for diffing
pub struct PaneInfo {
    pub id: u32,
    pub is_focused: bool,
    pub is_floating: bool,
    pub is_suppressed: bool,
    pub is_plugin: bool,
}

impl EventStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a subscriber. Returns a vec of events representing current state
    /// (so the subscriber gets an initial snapshot).
    pub fn subscribe(&mut self, pipe_id: String) -> Vec<Event> {
        let subscriber = Subscriber::new(pipe_id.clone());
        self.subscribers.insert(pipe_id, subscriber);

        let mut initial_events = Vec::new();

        // Send current focus state
        if let Some((pane_id, _)) = self.last_focused {
            initial_events.push(Event::PaneFocused { pane_id });
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
            // Still update internal state even without subscribers
            self.update_state(panes);
            return vec![];
        }

        let events = self.compute_events(panes);
        self.update_state(panes);

        // Broadcast each event to all subscribers
        let mut output = Vec::new();
        for event in &events {
            let json = event.to_json();
            for sub in self.subscribers.values() {
                output.push((sub.pipe_id.clone(), json.clone()));
            }
        }

        output
    }

    /// Compute the events that should be emitted for this pane update
    fn compute_events(&self, panes: &[PaneInfo]) -> Vec<Event> {
        let mut events = Vec::new();

        // 1. Detect pane opens/closes by diffing IDs
        let new_ids: HashSet<u32> = panes.iter().map(|p| p.id).collect();
        let old_ids = &self.known_panes;

        // Opened panes (in new but not old)
        for pane in panes {
            if !old_ids.contains(&pane.id) {
                events.push(Event::PaneOpened {
                    pane_id: pane.id,
                    is_floating: pane.is_floating,
                });
            }
        }

        // Closed panes (in old but not new)
        for &old_id in old_ids {
            if !new_ids.contains(&old_id) {
                events.push(Event::PaneClosed { pane_id: old_id });
            }
        }

        // 2. Detect focus changes
        let new_focused = Self::find_focused(panes);

        match (self.last_focused, new_focused) {
            (Some((old_id, _)), Some((new_id, _))) if old_id != new_id => {
                events.push(Event::PaneUnfocused { pane_id: old_id });
                events.push(Event::PaneFocused { pane_id: new_id });
            }
            (Some((old_id, _)), None) => {
                events.push(Event::PaneUnfocused { pane_id: old_id });
            }
            (None, Some((new_id, _))) => {
                events.push(Event::PaneFocused { pane_id: new_id });
            }
            _ => {} // Same pane or both None — no change
        }

        events
    }

    /// Update internal state to match current pane manifest
    fn update_state(&mut self, panes: &[PaneInfo]) {
        self.known_panes = panes.iter().map(|p| p.id).collect();
        self.last_focused = Self::find_focused(panes);
    }

    /// Find the focused pane from a list of panes.
    /// Floating panes take precedence. Suppressed panes are excluded.
    fn find_focused(panes: &[PaneInfo]) -> Option<(u32, bool)> {
        let mut focused_tiled: Option<(u32, bool)> = None;
        let mut focused_floating: Option<(u32, bool)> = None;

        for pane in panes {
            if pane.is_focused && !pane.is_suppressed {
                if pane.is_floating {
                    focused_floating = Some((pane.id, pane.is_plugin));
                } else {
                    focused_tiled = Some((pane.id, pane.is_plugin));
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
        }
    }

    fn make_suppressed_pane(id: u32, is_focused: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_focused,
            is_floating: true,
            is_suppressed: true,
            is_plugin: false,
        }
    }

    // --- Event serialization ---

    #[test]
    fn event_pane_focused_serializes() {
        let event = Event::PaneFocused { pane_id: 42 };
        assert_eq!(event.to_json(), r#"{"PaneFocused":{"pane_id":42}}"#);
    }

    #[test]
    fn event_pane_unfocused_serializes() {
        let event = Event::PaneUnfocused { pane_id: 42 };
        assert_eq!(event.to_json(), r#"{"PaneUnfocused":{"pane_id":42}}"#);
    }

    #[test]
    fn event_pane_opened_serializes() {
        let event = Event::PaneOpened {
            pane_id: 5,
            is_floating: true,
        };
        assert_eq!(
            event.to_json(),
            r#"{"PaneOpened":{"pane_id":5,"is_floating":true}}"#
        );
    }

    #[test]
    fn event_pane_closed_serializes() {
        let event = Event::PaneClosed { pane_id: 7 };
        assert_eq!(event.to_json(), r#"{"PaneClosed":{"pane_id":7}}"#);
    }

    // --- Subscriber management ---

    #[test]
    fn subscribe_with_no_prior_state_returns_empty() {
        let mut stream = EventStream::new();
        let events = stream.subscribe("pipe-1".to_string());
        assert!(events.is_empty());
    }

    #[test]
    fn subscribe_returns_current_focus() {
        let mut stream = EventStream::new();
        // Set up state with a focused pane
        let panes = vec![make_pane(42, true, false)];
        stream.on_pane_update(&panes);

        let events = stream.subscribe("pipe-1".to_string());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], Event::PaneFocused { pane_id: 42 });
    }

    #[test]
    fn unsubscribe_removes_subscriber() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string());
        assert!(stream.has_subscribers());
        stream.unsubscribe("pipe-1");
        assert!(!stream.has_subscribers());
    }

    // --- Focus change detection ---

    #[test]
    fn focus_change_emits_focused_event() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string());

        let panes = vec![make_pane(42, true, false)];
        let output = stream.on_pane_update(&panes);

        // Should have PaneOpened + PaneFocused
        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":42}}"#));
    }

    #[test]
    fn focus_switch_emits_unfocus_then_focus() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string());

        // Initial: pane 42 focused
        let panes = vec![make_pane(42, true, false), make_pane(17, false, false)];
        stream.on_pane_update(&panes);

        // Switch focus to pane 17
        let panes = vec![make_pane(42, false, false), make_pane(17, true, false)];
        let output = stream.on_pane_update(&panes);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneUnfocused":{"pane_id":42}}"#));
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":17}}"#));
    }

    #[test]
    fn same_focus_emits_no_focus_events() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string());

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
        stream.subscribe("pipe-1".to_string());

        // Initial: one pane
        let panes = vec![make_pane(1, true, false)];
        stream.on_pane_update(&panes);

        // New pane appears
        let panes = vec![make_pane(1, true, false), make_pane(2, false, true)];
        let output = stream.on_pane_update(&panes);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneOpened":{"pane_id":2,"is_floating":true}}"#));
    }

    #[test]
    fn removed_pane_emits_closed_event() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string());

        // Initial: two panes
        let panes = vec![make_pane(1, true, false), make_pane(2, false, false)];
        stream.on_pane_update(&panes);

        // Pane 2 disappears
        let panes = vec![make_pane(1, true, false)];
        let output = stream.on_pane_update(&panes);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneClosed":{"pane_id":2}}"#));
    }

    // --- Floating pane focus precedence ---

    #[test]
    fn floating_pane_takes_focus_precedence() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string());

        // Both tiled and floating report focused, floating wins
        let panes = vec![make_pane(1, true, false), make_pane(2, true, true)];
        let output = stream.on_pane_update(&panes);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":2}}"#));
        // Pane 1 should NOT get a focused event
        assert!(!jsons.contains(&r#"{"PaneFocused":{"pane_id":1}}"#));
    }

    #[test]
    fn suppressed_pane_not_considered_focused() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string());

        let panes = vec![make_pane(1, true, false), make_suppressed_pane(2, true)];
        let output = stream.on_pane_update(&panes);

        let jsons: Vec<&str> = output.iter().map(|(_, json)| json.as_str()).collect();
        // Pane 1 (tiled) should be focused since pane 2 is suppressed
        assert!(jsons.contains(&r#"{"PaneFocused":{"pane_id":1}}"#));
    }

    // --- Multiple subscribers ---

    #[test]
    fn events_broadcast_to_all_subscribers() {
        let mut stream = EventStream::new();
        stream.subscribe("pipe-1".to_string());
        stream.subscribe("pipe-2".to_string());

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
}
