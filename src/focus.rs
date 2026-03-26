use crate::message::ParseError;

pub enum FocusTabTarget {
    Position(u32),
    TabId(usize),
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
        ["id", tab_id] => tab_id
            .parse::<usize>()
            .map(FocusTabTarget::TabId)
            .map_err(|e| ParseError::InvalidArgs(format!("Invalid tab ID: {}", e))),
        _ => Err(ParseError::InvalidArgs(format!(
            "focus-tab requires 1 argument (position) or 2 arguments (position::<n> / id::<id>), got {}",
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
            FocusTabTarget::TabId(_) => panic!("expected position target"),
        }
    }

    #[test]
    fn parse_focus_tab_position_long_form() {
        let target = parse_focus_tab_target(&["position", "3"]).expect("should parse");
        match target {
            FocusTabTarget::Position(tab) => assert_eq!(tab, 3),
            FocusTabTarget::TabId(_) => panic!("expected position target"),
        }
    }

    #[test]
    fn parse_focus_tab_id_form() {
        let target = parse_focus_tab_target(&["id", "42"]).expect("should parse");
        match target {
            FocusTabTarget::TabId(tab) => assert_eq!(tab, 42),
            FocusTabTarget::Position(_) => panic!("expected tab ID target"),
        }
    }
}
