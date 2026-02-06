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
}
