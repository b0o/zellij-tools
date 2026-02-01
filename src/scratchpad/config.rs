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
