use std::collections::HashMap;

use serde::Serialize;

/// A focus change event emitted to subscribers
#[derive(Debug, Clone, Serialize)]
pub struct FocusEvent {
    pub pane_id: u32,
    pub focused: bool,
}

impl FocusEvent {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

/// A subscriber to focus events
#[derive(Debug, Clone)]
pub struct FocusSubscriber {
    /// The pipe ID to send events to
    pub pipe_id: String,
    /// Optional pane ID filter (None = all panes)
    pub pane_filter: Option<u32>,
}

impl FocusSubscriber {
    pub fn new(pipe_id: String, pane_filter: Option<u32>) -> Self {
        Self {
            pipe_id,
            pane_filter,
        }
    }

    /// Check if this subscriber wants events for the given pane
    pub fn wants_pane(&self, pane_id: u32) -> bool {
        self.pane_filter.map_or(true, |filter| filter == pane_id)
    }
}

/// Tracks focus state and manages subscribers
#[derive(Debug, Default)]
pub struct FocusTracker {
    /// Active subscribers keyed by pipe ID
    subscribers: HashMap<String, FocusSubscriber>,
    /// Last known focused pane (pane_id, is_plugin)
    last_focused: Option<(u32, bool)>,
}

impl FocusTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a subscriber, returns the current focus state to emit immediately
    pub fn subscribe(&mut self, pipe_id: String, pane_filter: Option<u32>) -> Option<FocusEvent> {
        let subscriber = FocusSubscriber::new(pipe_id.clone(), pane_filter);
        self.subscribers.insert(pipe_id, subscriber);

        // Return current state if we have it and subscriber wants it
        self.last_focused.and_then(|(pane_id, _is_plugin)| {
            if pane_filter.map_or(true, |f| f == pane_id) {
                Some(FocusEvent {
                    pane_id,
                    focused: true,
                })
            } else {
                None
            }
        })
    }

    /// Remove a subscriber
    pub fn unsubscribe(&mut self, pipe_id: &str) {
        self.subscribers.remove(pipe_id);
    }

    /// Get all subscribers that want events for a given pane
    pub fn subscribers_for_pane(&self, pane_id: u32) -> Vec<&FocusSubscriber> {
        self.subscribers
            .values()
            .filter(|s| s.wants_pane(pane_id))
            .collect()
    }

    /// Check if there are any subscribers
    pub fn has_subscribers(&self) -> bool {
        !self.subscribers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_event_serializes_to_json() {
        let event = FocusEvent {
            pane_id: 42,
            focused: true,
        };
        assert_eq!(event.to_json(), r#"{"pane_id":42,"focused":true}"#);
    }

    #[test]
    fn subscriber_with_no_filter_wants_all_panes() {
        let sub = FocusSubscriber::new("pipe-1".to_string(), None);
        assert!(sub.wants_pane(1));
        assert!(sub.wants_pane(42));
        assert!(sub.wants_pane(999));
    }

    #[test]
    fn subscriber_with_filter_only_wants_specific_pane() {
        let sub = FocusSubscriber::new("pipe-1".to_string(), Some(42));
        assert!(!sub.wants_pane(1));
        assert!(sub.wants_pane(42));
        assert!(!sub.wants_pane(999));
    }

    #[test]
    fn tracker_subscribe_returns_none_when_no_prior_focus() {
        let mut tracker = FocusTracker::new();
        let event = tracker.subscribe("pipe-1".to_string(), None);
        assert!(event.is_none());
    }

    #[test]
    fn tracker_unsubscribe_removes_subscriber() {
        let mut tracker = FocusTracker::new();
        tracker.subscribe("pipe-1".to_string(), None);
        assert!(tracker.has_subscribers());
        tracker.unsubscribe("pipe-1");
        assert!(!tracker.has_subscribers());
    }

    #[test]
    fn tracker_subscribers_for_pane_filters_correctly() {
        let mut tracker = FocusTracker::new();
        tracker.subscribe("all-panes".to_string(), None);
        tracker.subscribe("pane-42-only".to_string(), Some(42));
        tracker.subscribe("pane-99-only".to_string(), Some(99));

        let subs = tracker.subscribers_for_pane(42);
        assert_eq!(subs.len(), 2); // all-panes and pane-42-only

        let subs = tracker.subscribers_for_pane(1);
        assert_eq!(subs.len(), 1); // only all-panes
    }
}
