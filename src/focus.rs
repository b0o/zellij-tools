use crate::message::ParseError;

pub enum FocusTabTarget {
    Position(u32),
    StableId(u64),
}

pub fn parse_focus_tab_target(args: &[&str]) -> Result<FocusTabTarget, ParseError> {
    match args {
        [position] => position
            .parse::<u32>()
            .map(FocusTabTarget::Position)
            .map_err(|e| ParseError::InvalidArgs(format!("Invalid tab index: {}", e))),
        ["position", position] => position
            .parse::<u32>()
            .map(FocusTabTarget::Position)
            .map_err(|e| ParseError::InvalidArgs(format!("Invalid tab index: {}", e))),
        ["id", stable_id] => stable_id
            .parse::<u64>()
            .map(FocusTabTarget::StableId)
            .map_err(|e| ParseError::InvalidArgs(format!("Invalid tab stable ID: {}", e))),
        _ => Err(ParseError::InvalidArgs(format!(
            "focus-tab requires 1 argument (position) or 2 arguments (position::<n> / id::<stable_id>), got {}",
            args.len()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_focus_tab_target, FocusTabTarget};

    #[test]
    fn parse_focus_tab_position_short_form() {
        let target = parse_focus_tab_target(&["3"]).expect("should parse");
        match target {
            FocusTabTarget::Position(tab) => assert_eq!(tab, 3),
            FocusTabTarget::StableId(_) => panic!("expected position target"),
        }
    }

    #[test]
    fn parse_focus_tab_position_long_form() {
        let target = parse_focus_tab_target(&["position", "3"]).expect("should parse");
        match target {
            FocusTabTarget::Position(tab) => assert_eq!(tab, 3),
            FocusTabTarget::StableId(_) => panic!("expected position target"),
        }
    }

    #[test]
    fn parse_focus_tab_stable_id_form() {
        let target = parse_focus_tab_target(&["id", "42"]).expect("should parse");
        match target {
            FocusTabTarget::StableId(tab) => assert_eq!(tab, 42),
            FocusTabTarget::Position(_) => panic!("expected stable ID target"),
        }
    }
}
