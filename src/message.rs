/// Errors that can occur when parsing pipe messages
#[derive(Debug)]
pub enum ParseError {
    InvalidFormat,
    WrongPlugin,
    UnknownEvent(String),
    InvalidArgs(String),
    InvalidScratchpadName(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::InvalidFormat => {
                write!(f, "Message format should be 'plugin::event::args...'")
            }
            ParseError::WrongPlugin => write!(f, "Message not intended for zellij-tools"),
            ParseError::UnknownEvent(event) => write!(f, "Unknown event: {}", event),
            ParseError::InvalidArgs(msg) => write!(f, "Invalid arguments: {}", msg),
            ParseError::InvalidScratchpadName(name) => {
                write!(
                    f,
                    "Invalid scratchpad name '{}': must match [a-zA-Z0-9_-]+",
                    name
                )
            }
        }
    }
}

/// A parsed pipe message
pub struct Message<'a> {
    pub event: &'a str,
    pub args: Vec<&'a str>,
}

/// Parse a pipe message payload into event and args.
/// Format: "zellij-tools::event::arg1::arg2::..."
pub fn parse_message(payload: &str) -> Result<Message<'_>, ParseError> {
    let mut parts = payload.splitn(3, "::");

    let plugin = parts.next().ok_or(ParseError::InvalidFormat)?;
    let event = parts.next().ok_or(ParseError::InvalidFormat)?;
    let args_str = parts.next().unwrap_or("");

    if plugin != "zellij-tools" {
        return Err(ParseError::WrongPlugin);
    }

    let args: Vec<&str> = if args_str.is_empty() {
        Vec::new()
    } else {
        args_str.split("::").collect()
    };

    Ok(Message { event, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_message_with_no_args() {
        let msg = parse_message("zellij-tools::focus-pane").unwrap();
        assert_eq!(msg.event, "focus-pane");
        assert!(msg.args.is_empty());
    }

    #[test]
    fn parse_message_with_one_arg() {
        let msg = parse_message("zellij-tools::scratchpad::toggle").unwrap();
        assert_eq!(msg.event, "scratchpad");
        assert_eq!(msg.args, vec!["toggle"]);
    }

    #[test]
    fn parse_message_with_multiple_args() {
        let msg = parse_message("zellij-tools::scratchpad::register::mypad::123").unwrap();
        assert_eq!(msg.event, "scratchpad");
        assert_eq!(msg.args, vec!["register", "mypad", "123"]);
    }

    #[test]
    fn parse_message_wrong_plugin() {
        let result = parse_message("other-plugin::event");
        assert!(matches!(result, Err(ParseError::WrongPlugin)));
    }

    #[test]
    fn parse_message_invalid_format_no_event() {
        let result = parse_message("zellij-tools");
        assert!(matches!(result, Err(ParseError::InvalidFormat)));
    }

    #[test]
    fn parse_message_invalid_format_empty() {
        let result = parse_message("");
        assert!(matches!(result, Err(ParseError::InvalidFormat)));
    }

    #[test]
    fn parse_subscribe() {
        let msg = parse_message("zellij-tools::subscribe").unwrap();
        assert_eq!(msg.event, "subscribe");
        assert!(msg.args.is_empty());
    }

    #[test]
    fn parse_unsubscribe() {
        let msg = parse_message("zellij-tools::unsubscribe::pipe-123").unwrap();
        assert_eq!(msg.event, "unsubscribe");
        assert_eq!(msg.args, vec!["pipe-123"]);
    }

    #[test]
    fn parse_tree() {
        let msg = parse_message("zellij-tools::tree").unwrap();
        assert_eq!(msg.event, "tree");
        assert!(msg.args.is_empty());
    }
}
