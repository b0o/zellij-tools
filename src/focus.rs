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
}
