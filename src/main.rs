use std::collections::BTreeMap;

use zellij_tile::prelude::*;

#[derive(Default)]
struct State {}

register_plugin!(State);

#[derive(Debug)]
enum ParseError {
    InvalidFormat,
    WrongPlugin,
    UnknownEvent(String),
    InvalidArgs(String),
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
        }
    }
}

impl State {
    fn parse_message(&self, payload: &str) -> Result<(String, Vec<String>), ParseError> {
        let mut parts = payload.splitn(3, "::");

        let plugin = parts.next().ok_or(ParseError::InvalidFormat)?;
        let event = parts.next().ok_or(ParseError::InvalidFormat)?;
        let args_str = parts.next().unwrap_or("");

        if plugin != "zellij-tools" {
            return Err(ParseError::WrongPlugin);
        }

        let args: Vec<String> = if args_str.is_empty() {
            Vec::new()
        } else {
            args_str.split("::").map(|s| s.to_string()).collect()
        };

        Ok((event.to_string(), args))
    }

    fn handle_event(&self, event: &str, args: Vec<String>) -> Result<(), ParseError> {
        match event {
            "focus-pane" => {
                if args.len() != 1 {
                    return Err(ParseError::InvalidArgs(format!(
                        "focus-pane requires 1 argument, got {}",
                        args.len()
                    )));
                }

                let pane_id = args[0]
                    .parse::<PaneId>()
                    .map_err(|e| ParseError::InvalidArgs(format!("Invalid pane ID: {}", e)))?;

                show_pane_with_id(pane_id, true);
                Ok(())
            }
            _ => Err(ParseError::UnknownEvent(event.to_string())),
        }
    }
}

impl ZellijPlugin for State {
    fn load(&mut self, _configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
        ]);
    }

    fn pipe(&mut self, pipe_message: PipeMessage) -> bool {
        let payload = match pipe_message.payload {
            Some(p) => p,
            None => return false,
        };

        match self.parse_message(&payload) {
            Ok((event, args)) => match self.handle_event(&event, args) {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("Error handling event: {}", e);
                    false
                }
            },
            Err(e) => {
                eprintln!("Error parsing message: {}", e);
                false
            }
        }
    }
}
