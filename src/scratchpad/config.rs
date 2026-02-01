use std::collections::HashMap;

use serde::Deserialize;

use crate::message::ParseError;

/// Configuration for a scratchpad
#[derive(Debug, Clone, Deserialize)]
pub struct ScratchpadConfig {
    pub command: Vec<String>,
}

/// Actions that can be performed on scratchpads
#[derive(Debug)]
pub enum ScratchpadAction {
    Toggle { name: Option<String> },
    Show { name: String },
    Hide { name: String },
    Close { name: String },
    Register { name: String, pane_id: u32 },
}

/// Check if a scratchpad name is valid (alphanumeric, underscore, hyphen)
pub fn is_valid_scratchpad_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parse a scratchpad action from message args
pub fn parse_scratchpad_action(args: &[String]) -> Result<ScratchpadAction, ParseError> {
    let action = args.first().map(|s| s.as_str()).unwrap_or("");

    match action {
        "toggle" => {
            let name = args.get(1).cloned();
            if let Some(ref n) = name {
                if !is_valid_scratchpad_name(n) {
                    return Err(ParseError::InvalidScratchpadName(n.clone()));
                }
            }
            Ok(ScratchpadAction::Toggle { name })
        }
        "show" => {
            let name = args.get(1).ok_or_else(|| {
                ParseError::InvalidArgs("show requires a scratchpad name".to_string())
            })?;
            if !is_valid_scratchpad_name(name) {
                return Err(ParseError::InvalidScratchpadName(name.clone()));
            }
            Ok(ScratchpadAction::Show { name: name.clone() })
        }
        "hide" => {
            let name = args.get(1).ok_or_else(|| {
                ParseError::InvalidArgs("hide requires a scratchpad name".to_string())
            })?;
            if !is_valid_scratchpad_name(name) {
                return Err(ParseError::InvalidScratchpadName(name.clone()));
            }
            Ok(ScratchpadAction::Hide { name: name.clone() })
        }
        "close" => {
            let name = args.get(1).ok_or_else(|| {
                ParseError::InvalidArgs("close requires a scratchpad name".to_string())
            })?;
            if !is_valid_scratchpad_name(name) {
                return Err(ParseError::InvalidScratchpadName(name.clone()));
            }
            Ok(ScratchpadAction::Close { name: name.clone() })
        }
        "register" => {
            // Format: register::<name>::<pane_id>
            let name = args
                .get(1)
                .ok_or_else(|| ParseError::InvalidArgs("register requires a name".to_string()))?;
            let pane_id_str = args.get(2).ok_or_else(|| {
                ParseError::InvalidArgs("register requires a pane_id".to_string())
            })?;
            let pane_id = pane_id_str.parse::<u32>().map_err(|e| {
                ParseError::InvalidArgs(format!("Invalid pane_id '{}': {}", pane_id_str, e))
            })?;
            if !is_valid_scratchpad_name(name) {
                return Err(ParseError::InvalidScratchpadName(name.clone()));
            }
            Ok(ScratchpadAction::Register {
                name: name.clone(),
                pane_id,
            })
        }
        _ => Err(ParseError::InvalidArgs(format!(
            "Unknown scratchpad action: {}",
            action
        ))),
    }
}

/// Parse scratchpad configurations from KDL format
pub fn parse_scratchpads_kdl(input: &str) -> Result<HashMap<String, ScratchpadConfig>, String> {
    use kdl::KdlDocument;

    let doc: KdlDocument = input
        .parse()
        .map_err(|e| format!("KDL parse error: {}", e))?;

    let mut configs = HashMap::new();

    for node in doc.nodes() {
        let name = node.name().value().to_string();

        if !is_valid_scratchpad_name(&name) {
            return Err(format!("Invalid scratchpad name: '{}'", name));
        }

        // Look for command child node
        let command = node
            .children()
            .and_then(|c| c.get("command"))
            .map(|cmd_node| {
                cmd_node
                    .entries()
                    .iter()
                    .filter_map(|e| e.value().as_string())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if command.is_empty() {
            return Err(format!("Scratchpad '{}' has no command", name));
        }

        configs.insert(name, ScratchpadConfig { command });
    }

    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;

    // is_valid_scratchpad_name tests
    #[test]
    fn valid_name_alphanumeric() {
        assert!(is_valid_scratchpad_name("terminal1"));
    }

    #[test]
    fn valid_name_with_underscore() {
        assert!(is_valid_scratchpad_name("my_scratchpad"));
    }

    #[test]
    fn valid_name_with_hyphen() {
        assert!(is_valid_scratchpad_name("my-scratchpad"));
    }

    #[test]
    fn invalid_name_empty() {
        assert!(!is_valid_scratchpad_name(""));
    }

    #[test]
    fn invalid_name_with_space() {
        assert!(!is_valid_scratchpad_name("my pad"));
    }

    #[test]
    fn invalid_name_with_special_chars() {
        assert!(!is_valid_scratchpad_name("my@pad"));
    }

    // parse_scratchpad_action tests
    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_toggle_no_name() {
        let action = parse_scratchpad_action(&args(&["toggle"])).unwrap();
        assert!(matches!(action, ScratchpadAction::Toggle { name: None }));
    }

    #[test]
    fn parse_toggle_with_name() {
        let action = parse_scratchpad_action(&args(&["toggle", "mypad"])).unwrap();
        assert!(matches!(
            action,
            ScratchpadAction::Toggle { name: Some(n) } if n == "mypad"
        ));
    }

    #[test]
    fn parse_show() {
        let action = parse_scratchpad_action(&args(&["show", "mypad"])).unwrap();
        assert!(matches!(
            action,
            ScratchpadAction::Show { name } if name == "mypad"
        ));
    }

    #[test]
    fn parse_show_missing_name() {
        let result = parse_scratchpad_action(&args(&["show"]));
        assert!(matches!(result, Err(ParseError::InvalidArgs(_))));
    }

    #[test]
    fn parse_hide() {
        let action = parse_scratchpad_action(&args(&["hide", "mypad"])).unwrap();
        assert!(matches!(
            action,
            ScratchpadAction::Hide { name } if name == "mypad"
        ));
    }

    #[test]
    fn parse_close() {
        let action = parse_scratchpad_action(&args(&["close", "mypad"])).unwrap();
        assert!(matches!(
            action,
            ScratchpadAction::Close { name } if name == "mypad"
        ));
    }

    #[test]
    fn parse_register() {
        let action = parse_scratchpad_action(&args(&["register", "mypad", "42"])).unwrap();
        assert!(matches!(
            action,
            ScratchpadAction::Register { name, pane_id } if name == "mypad" && pane_id == 42
        ));
    }

    #[test]
    fn parse_register_missing_pane_id() {
        let result = parse_scratchpad_action(&args(&["register", "mypad"]));
        assert!(matches!(result, Err(ParseError::InvalidArgs(_))));
    }

    #[test]
    fn parse_register_invalid_pane_id() {
        let result = parse_scratchpad_action(&args(&["register", "mypad", "notanumber"]));
        assert!(matches!(result, Err(ParseError::InvalidArgs(_))));
    }

    #[test]
    fn parse_invalid_name() {
        let result = parse_scratchpad_action(&args(&["show", "my pad"]));
        assert!(matches!(result, Err(ParseError::InvalidScratchpadName(_))));
    }

    #[test]
    fn parse_unknown_action() {
        let result = parse_scratchpad_action(&args(&["unknown"]));
        assert!(matches!(result, Err(ParseError::InvalidArgs(_))));
    }

    // parse_scratchpads_kdl tests
    #[test]
    fn parse_kdl_single_scratchpad() {
        let input = r#"term { command "nu"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs["term"].command, vec!["nu"]);
    }

    #[test]
    fn parse_kdl_multi_arg_command() {
        let input = r#"claude { command "direnv" "exec" "." "claude-bun"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(
            configs["claude"].command,
            vec!["direnv", "exec", ".", "claude-bun"]
        );
    }

    #[test]
    fn parse_kdl_multiple_scratchpads() {
        let input = r#"
            term { command "nu"; }
            htop { command "htop"; }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs.len(), 2);
    }

    #[test]
    fn parse_kdl_error_on_missing_command() {
        let input = r#"term { }"#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    #[test]
    fn parse_kdl_error_on_invalid_name() {
        let input = r#"my pad { command "nu"; }"#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }
}
