//! One ephemeral, validated status per live pane. No state is persisted.
//!
//! Untrusted opening braces become `{ `: receiver click routing searches already
//! expanded output for widget tokens, even without recursive template expansion.

use std::collections::{BTreeMap, HashMap, HashSet};

use zellij_tile::prelude::{PaneId, PaneInfo, TabInfo};

const MAX_STATUS_BYTES: usize = 4096;
const MAX_STYLE_MARKERS: usize = 64;

/// Pane-owned render-ready text, retained until cleared or authoritatively closed.
#[derive(Debug, Default)]
pub struct PaneStatuses {
    statuses: HashMap<PaneId, String>,
}

/// Statuses grouped by native tab ID, including tabs without any statuses.
#[derive(Clone, Debug, Default)]
pub struct PaneStatusSnapshot {
    pub current_tab_id: Option<usize>,
    pub tabs: BTreeMap<usize, Vec<PaneStatusItem>>,
}

/// Live pane metadata and safe, render-ready zjstatus text.
#[derive(Clone, Debug)]
pub struct PaneStatusItem {
    pub pane_id: PaneId,
    pub title: String,
    pub status: String,
    pub is_focused: bool,
}

impl PaneStatuses {
    /// Replace a known pane's status, or clear it with an empty message.
    ///
    /// `args` must contain exactly `[typed_pane_id, message]`; the caller must
    /// preserve any `::` separators inside the message. Limits apply to input
    /// bytes and input style markers, excluding the generated isolation resets.
    ///
    /// # Errors
    /// Rejects invalid arguments, unknown panes, unsupported formats, oversized
    /// text, controls, and unsafe markup. Errors leave existing state unchanged.
    pub fn set(
        &mut self,
        args: &[&str],
        format: Option<&str>,
        manifest: &HashMap<usize, Vec<PaneInfo>>,
    ) -> Result<(), String> {
        let [raw_id, message] = args else {
            return Err("pane status requires exactly a typed pane ID and a message".into());
        };
        let formatted = match format {
            None | Some("plain") => false,
            Some("zjstatus") => true,
            Some(_) => return Err("pane status format must be plain or zjstatus".into()),
        };
        let digits = raw_id
            .strip_prefix("terminal_")
            .or_else(|| raw_id.strip_prefix("plugin_"))
            .ok_or("pane ID must be terminal_N or plugin_N")?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("pane ID must end with ASCII decimal digits only".into());
        }
        let id = raw_id
            .parse::<PaneId>()
            .map_err(|_| "invalid pane ID".to_string())?;
        if !manifest.values().flatten().any(|pane| pane_id(pane) == id) {
            return Err("pane ID is not present in the current manifest".into());
        }
        let status = render_status(message, formatted)?;
        if status.is_empty() {
            self.statuses.remove(&id);
        } else {
            self.statuses.insert(id, status);
        }
        Ok(())
    }

    /// Remove absent pane IDs. Call only with an authoritative PaneUpdate.
    pub fn reconcile(&mut self, manifest: &HashMap<usize, Vec<PaneInfo>>) {
        let live: HashSet<_> = manifest.values().flatten().map(pane_id).collect();
        self.statuses.retain(|id, _| live.contains(id));
    }

    /// Read current metadata without pruning state during partial event updates.
    ///
    /// Manifest keys are tab positions, not tab IDs. Panes without a status and
    /// manifest positions without a matching tab are omitted. Within each tab,
    /// terminals precede plugins, each ordered by numeric pane ID.
    pub fn snapshot(
        &self,
        manifest: &HashMap<usize, Vec<PaneInfo>>,
        tabs: &[TabInfo],
    ) -> PaneStatusSnapshot {
        let mut snapshot = PaneStatusSnapshot {
            current_tab_id: tabs.iter().find(|tab| tab.active).map(|tab| tab.tab_id),
            ..Default::default()
        };
        for tab in tabs {
            let mut items: Vec<_> = manifest
                .get(&tab.position)
                .into_iter()
                .flatten()
                .filter_map(|pane| {
                    let pane_id = pane_id(pane);
                    self.statuses.get(&pane_id).map(|status| PaneStatusItem {
                        pane_id,
                        title: escape_text(&pane.title),
                        status: status.clone(),
                        is_focused: pane.is_focused,
                    })
                })
                .collect();
            items.sort_by_key(|item| match item.pane_id {
                PaneId::Terminal(id) => (false, id),
                PaneId::Plugin(id) => (true, id),
            });
            snapshot.tabs.insert(tab.tab_id, items);
        }
        snapshot
    }
}

fn pane_id(pane: &PaneInfo) -> PaneId {
    if pane.is_plugin {
        PaneId::Plugin(pane.id)
    } else {
        PaneId::Terminal(pane.id)
    }
}

fn unsafe_character(character: char) -> bool {
    character.is_control()
        || matches!(character,
            '\u{00ad}' | '\u{061c}' | '\u{180e}'
            | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}'
            | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')
}

/// Sanitize plain text for zjstatus: replace controls, bidi and invisible
/// formatting characters with spaces, and neutralize `#[` as `# [`.
/// Add a space after every `{`, preserving `}`, so receiver click routing cannot
/// mistake literal text for widget tokens, including across substitutions.
pub fn escape_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if unsafe_character(character) {
            escaped.push(' ');
        } else {
            if character == '[' && escaped.ends_with('#') {
                escaped.push(' ');
            }
            escaped.push(character);
            if character == '{' {
                escaped.push(' ');
            }
        }
    }
    escaped
}

fn render_status(message: &str, formatted: bool) -> Result<String, String> {
    if message.len() > MAX_STATUS_BYTES {
        return Err("pane status exceeds 4096 UTF-8 bytes".into());
    }
    if message.chars().any(unsafe_character) {
        return Err("pane status contains unsafe control or formatting characters".into());
    }
    if message.is_empty() || !formatted {
        return Ok(escape_text(message));
    }

    let mut rest = message;
    let mut markers = 0;
    while let Some(start) = rest.find("#[") {
        rest = &rest[start + 2..];
        let end = rest
            .find(']')
            .ok_or("unterminated pane status style marker")?;
        markers += 1;
        if markers > MAX_STYLE_MARKERS {
            return Err("pane status exceeds 64 style markers".into());
        }
        let style = &rest[..end];
        if !style.is_empty() {
            for directive in style.split(',') {
                if matches!(directive, "bold" | "italic" | "underscore") {
                    continue;
                }
                let color = directive
                    .strip_prefix("fg=")
                    .or_else(|| directive.strip_prefix("bg="))
                    .ok_or("unsupported pane status style directive")?;
                if !valid_color(color) {
                    return Err("invalid pane status color".into());
                }
            }
        }
        rest = &rest[end + 1..];
    }

    // Neutralize click tokens only after validation, preserving accepted styles.
    let message = message.replace('{', "{ ");
    // Resets isolate both the first unstyled segment and subsequent consumer text.
    Ok(format!("#[]{message}#[]"))
}

fn valid_color(color: &str) -> bool {
    if let Some(hex) = color.strip_prefix('#') {
        // Check bytes before zjstatus can slice a non-ASCII string at byte offsets.
        return hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    if !color.is_empty() && color.bytes().all(|byte| byte.is_ascii_digit()) {
        return color.parse::<u8>().is_ok();
    }
    matches!(
        color.strip_prefix("bright_").unwrap_or(color),
        "black" | "red" | "green" | "yellow" | "blue" | "magenta" | "cyan" | "white"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u32, is_plugin: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_plugin,
            title: format!("pane {id}"),
            ..Default::default()
        }
    }

    fn tab(tab_id: usize, position: usize, active: bool) -> TabInfo {
        TabInfo {
            tab_id,
            position,
            active,
            ..Default::default()
        }
    }

    fn manifest() -> HashMap<usize, Vec<PaneInfo>> {
        HashMap::from([(0, vec![pane(2, false), pane(2, true)])])
    }

    #[test]
    fn defaults_and_empty_tabs() {
        let default = PaneStatusSnapshot::default();
        assert_eq!(default.current_tab_id, None);
        assert!(default.tabs.is_empty());
        let statuses = PaneStatuses::default();
        let snapshot = statuses.snapshot(&manifest(), &[tab(42, 0, true), tab(9, 1, false)]);
        assert_eq!(snapshot.current_tab_id, Some(42));
        assert_eq!(snapshot.tabs.keys().copied().collect::<Vec<_>>(), [9, 42]);
        assert!(snapshot.tabs.values().all(Vec::is_empty));
        assert_eq!(statuses.snapshot(&manifest(), &[]).current_tab_id, None);
    }

    #[test]
    fn validates_argument_count_and_preserves_message_separators() {
        let mut statuses = PaneStatuses::default();
        for args in [vec![], vec!["terminal_2"], vec!["terminal_2", "a", "b"]] {
            assert!(statuses.set(&args, None, &manifest()).is_err());
        }
        // Integration must split the pane/message boundary only, not the text.
        let args: Vec<_> = "terminal_2::build::step::done::".splitn(2, "::").collect();
        statuses.set(&args, None, &manifest()).unwrap();
        assert_eq!(
            statuses.statuses[&PaneId::Terminal(2)],
            "build::step::done::"
        );
    }

    #[test]
    fn strict_typed_ids_and_unknown_panes() {
        let mut statuses = PaneStatuses::default();
        for id in [
            "2",
            "terminal",
            "terminal_",
            "plugin_",
            "terminal_+2",
            "terminal_-2",
            "terminal_2junk",
            "terminal_2_extra",
            "terminal_2::x",
            "terminal_2 ",
            " terminal_2",
            "Terminal_2",
            "plugin_2junk",
            "terminal_\u{ff12}",
            "terminal_4294967296",
            "terminal_999",
            "plugin_999",
        ] {
            for message in ["status", ""] {
                assert!(
                    statuses.set(&[id, message], None, &manifest()).is_err(),
                    "{id:?}"
                );
            }
        }
        assert!(statuses.statuses.is_empty());
        statuses
            .set(&["terminal_2", "terminal"], None, &manifest())
            .unwrap();
        statuses
            .set(&["plugin_2", "plugin"], None, &manifest())
            .unwrap();
        assert_eq!(statuses.statuses.len(), 2);
        assert_eq!(statuses.statuses[&PaneId::Terminal(2)], "terminal");
        assert_eq!(statuses.statuses[&PaneId::Plugin(2)], "plugin");
        assert!(statuses
            .set(&["terminal_2", "x"], None, &HashMap::new())
            .is_err());
    }

    #[test]
    fn replace_clear_and_errors_are_atomic() {
        let mut statuses = PaneStatuses::default();
        statuses
            .set(&["terminal_2", "first"], None, &manifest())
            .unwrap();
        statuses
            .set(&["terminal_2", "second"], Some("plain"), &manifest())
            .unwrap();
        for format in ["", "ansi", "Plain", "ZJSTATUS"] {
            for text in ["replacement", ""] {
                assert!(statuses
                    .set(&["terminal_2", text], Some(format), &manifest())
                    .is_err());
            }
        }
        for (text, format) in [("#[bad]x", Some("zjstatus")), ("bad\n", None)] {
            assert!(statuses
                .set(&["terminal_2", text], format, &manifest())
                .is_err());
            assert_eq!(statuses.statuses[&PaneId::Terminal(2)], "second");
        }
        for format in [None, Some("plain"), Some("zjstatus")] {
            statuses
                .set(&["terminal_2", "value"], format, &manifest())
                .unwrap();
            statuses
                .set(&["terminal_2", ""], format, &manifest())
                .unwrap();
            statuses
                .set(&["terminal_2", ""], format, &manifest())
                .unwrap();
            assert!(statuses.statuses.is_empty());
        }
    }

    #[test]
    fn plain_text_neutralizes_markup_and_opening_braces() {
        let text = "#[fg=red]#{name} {command_bad} ##[ #[unterminated";
        let expected = "# [fg=red]#{ name} { command_bad} ## [ # [unterminated";
        assert_eq!(render_status(text, false).unwrap(), expected);
        assert_eq!(escape_text(text), expected);
        assert_eq!(escape_text(""), "");
        assert_eq!(render_status("", true).unwrap(), "");
        assert_eq!(render_status("", false).unwrap(), "");
    }

    #[test]
    fn statuses_and_snapshot_titles_neutralize_click_tokens() {
        for (text, expected) in [
            ("{command_action}", "{ command_action}"),
            ("{pipe_status}", "{ pipe_status}"),
            ("{tabs}", "{ tabs}"),
            ("{", "{ "),
            ("{{tabs}{pipe_status}}", "{ { tabs}{ pipe_status}}"),
            ("{ tabs}", "{  tabs}"),
            ("}", "}"),
        ] {
            for format in [None, Some("plain"), Some("zjstatus")] {
                let mut manifest = manifest();
                manifest.get_mut(&0).unwrap()[0].title = text.into();
                let mut statuses = PaneStatuses::default();
                statuses
                    .set(&["terminal_2", text], format, &manifest)
                    .unwrap();
                let snapshot = statuses.snapshot(&manifest, &[tab(42, 0, true)]);
                let item = &snapshot.tabs[&42][0];
                assert_eq!(item.title, expected);
                assert_eq!(
                    item.status,
                    if format == Some("zjstatus") {
                        format!("#[]{expected}#[]")
                    } else {
                        expected.into()
                    }
                );
            }
        }
    }

    #[test]
    fn click_tokens_stay_neutralized_across_styles_and_substitutions() {
        let text =
            "#[fg=red,bold]{command_action}#[]{#[italic]pipe_status}#[bg=blue,underscore]{tabs}";
        assert_eq!(
            render_status(text, true).unwrap(),
            "#[]#[fg=red,bold]{ command_action}#[]{ #[italic]pipe_status}#[bg=blue,underscore]{ tabs}#[]"
        );
        for formatted in [false, true] {
            for token in ["{command_action}", "{pipe_status}", "{tabs}"] {
                for split in 1..token.len() {
                    let left = render_status(&token[..split], formatted).unwrap();
                    let right = render_status(&token[split..], formatted).unwrap();
                    // Model the visible text after isolation resets are rendered.
                    let visible = format!("{left}{right}").replace("#[]", "");
                    assert_eq!(visible, token.replace('{', "{ "));
                    assert!(!visible.contains(token));
                    assert!(!format!("{}{right}", escape_text(&token[..split]))
                        .replace("#[]", "")
                        .contains(token));
                }
            }
        }
        assert!(render_status("{tabs}#[fg={red}]x", true).is_err());
    }

    #[test]
    fn accepts_only_exact_supported_styles_and_colors() {
        for name in [
            "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
        ] {
            for color in [name.to_string(), format!("bright_{name}")] {
                for field in ["fg", "bg"] {
                    let text = format!("#[{field}={color},bold,italic,underscore]text");
                    assert_eq!(render_status(&text, true).unwrap(), format!("#[]{text}#[]"));
                }
            }
        }
        for color in 0..=255 {
            assert!(render_status(&format!("#[fg={color}]x"), true).is_ok());
        }
        for text in [
            "#[]",
            "#[bold]x",
            "#[italic]x",
            "#[underscore]x",
            "#[fg=#aAbB09,bg=#000000]x",
            "before#[fg=red]middle#[]after",
            "text {name} :: \u{1f680}",
            "text#",
            "text]",
        ] {
            assert_eq!(
                render_status(text, true).unwrap(),
                format!("#[]{}#[]", text.replace('{', "{ "))
            );
        }
    }

    #[test]
    fn rejects_malformed_markup_directives_and_colors() {
        for text in [
            "#[",
            "#[fg=red",
            "ok#[bold]x#[",
            "#[fg=red#[bold]]x",
            "#[[bold]]x",
            "#[,]x",
            "#[bold,]x",
            "#[,bold]x",
            "#[bold,,italic]x",
            "#[ ]x",
            "#[bold=true]x",
            "#[underline]x",
            "#[blink]x",
            "#[hidden]x",
            "#[reverse]x",
            "#[dim]x",
            "#[strikethrough]x",
            "#[us=red]x",
            "#[fg=red, italic]x",
            "#[FG=red]x",
            "#[fg=]x",
            "#[fg=Red]x",
            "#[fg=bright_bright_red]x",
            "#[fg=gray]x",
            "#[fg=default]x",
            "#[fg=$alias]x",
            "#[fg=colour2]x",
            "#[fg=256]x",
            "#[fg=-1]x",
            "#[fg=+1]x",
            "#[fg=1.0]x",
            "#[fg=0xff]x",
            "#[fg=999999999999999999999]x",
            "#[fg=\u{ff11}]x",
            "#[fg= 2]x",
            "#[fg=#123]x",
            "#[fg=#1234567]x",
            "#[fg=#gg0000]x",
            "#[bg=#]x",
            "#[fg=#a\u{e9}000]x",
            "#[fg=#\u{ff10}\u{ff10}]x",
            "#[fg=red=blue]x",
        ] {
            assert!(render_status(text, true).is_err(), "accepted {text:?}");
        }
    }

    #[test]
    fn rejects_controls_and_sanitizes_titles() {
        let characters = (0..=0x1f)
            .chain(0x7f..=0x9f)
            .chain([0xad, 0x61c, 0x180e, 0xfeff, 0xfff9, 0xfffa, 0xfffb, 0xe0001])
            .chain(0x200b..=0x200f)
            .chain(0x2028..=0x202e)
            .chain(0x2060..=0x206f)
            .chain(0xe0020..=0xe007f);
        for code in characters {
            let character = char::from_u32(code).unwrap();
            let text = format!("left{character}right");
            for formatted in [false, true] {
                assert!(
                    render_status(&text, formatted).is_err(),
                    "accepted U+{code:04X}"
                );
            }
            assert_eq!(escape_text(&text), "left right");
        }
        assert_eq!(escape_text("#\n[fg=red]\r\u{1b}[31m"), "# [fg=red]  [31m");
        let unicode = "caf\u{e9} \u{65e5}\u{672c} \u{1f680}";
        assert_eq!(escape_text(unicode), unicode);
        assert_eq!(render_status(unicode, false).unwrap(), unicode);
    }

    #[test]
    fn limits_input_bytes_and_complete_markers() {
        for formatted in [false, true] {
            assert!(render_status(&"a".repeat(4096), formatted).is_ok());
            assert!(render_status(&"a".repeat(4097), formatted).is_err());
            assert!(render_status(&"\u{e9}".repeat(2048), formatted).is_ok());
            assert!(render_status(&"\u{e9}".repeat(2049), formatted).is_err());
        }
        assert!(render_status(&"#[]".repeat(64), true).is_ok());
        assert!(render_status(&"#[]".repeat(65), true).is_err());
        assert!(render_status(&"#[bold]".repeat(64), true).is_ok());
        assert!(render_status(&"#[bold]".repeat(65), true).is_err());
        assert!(render_status(&format!("{}#[", "#[]".repeat(63)), true).is_err());
        assert!(render_status(&"#[".repeat(65), false).is_ok());
    }

    #[test]
    fn snapshot_tracks_moves_focus_titles_tab_reordering_and_closure() {
        let mut manifest = manifest();
        let mut statuses = PaneStatuses::default();
        statuses
            .set(
                &["terminal_2", "#[fg=red]working"],
                Some("zjstatus"),
                &manifest,
            )
            .unwrap();
        statuses
            .set(&["plugin_2", "plugin"], None, &manifest)
            .unwrap();
        let mut moved = manifest.get_mut(&0).unwrap().remove(0);
        moved.is_focused = true;
        moved.is_suppressed = true;
        moved.is_floating = true;
        moved.exited = true;
        moved.is_held = true;
        moved.title = "new\n#[bold]title".into();
        manifest.insert(1, vec![moved]);
        statuses.reconcile(&manifest);
        let tabs = [tab(42, 0, false), tab(9, 1, true), tab(77, 2, false)];
        let snapshot = statuses.snapshot(&manifest, &tabs);
        assert_eq!(snapshot.current_tab_id, Some(9));
        assert_eq!(snapshot.tabs[&42][0].pane_id, PaneId::Plugin(2));
        assert!(!snapshot.tabs[&42][0].is_focused);
        let item = &snapshot.tabs[&9][0];
        assert_eq!(item.pane_id, PaneId::Terminal(2));
        assert_eq!(item.title, "new # [bold]title");
        assert_eq!(item.status, "#[]#[fg=red]working#[]");
        assert!(item.is_focused);
        assert!(snapshot.tabs[&77].is_empty());

        let reordered = HashMap::from([(0, manifest[&1].clone()), (1, manifest[&0].clone())]);
        let tabs = [tab(9, 0, false), tab(42, 1, true)];
        let snapshot = statuses.snapshot(&reordered, &tabs);
        assert_eq!(snapshot.current_tab_id, Some(42));
        assert_eq!(snapshot.tabs[&9][0].pane_id, PaneId::Terminal(2));
        assert_eq!(snapshot.tabs[&42][0].pane_id, PaneId::Plugin(2));

        let closed = HashMap::from([(1, vec![pane(2, true)])]);
        statuses.reconcile(&closed);
        assert_eq!(statuses.statuses.len(), 1);
        assert!(statuses.snapshot(&closed, &tabs).tabs[&9].is_empty());
        // Reused numeric IDs must not inherit a closed pane's status.
        assert!(statuses.snapshot(&reordered, &tabs).tabs[&9].is_empty());
        statuses.reconcile(&HashMap::new());
        assert!(statuses.statuses.is_empty());
    }

    #[test]
    fn snapshots_do_not_prune_on_missing_metadata() {
        let mut statuses = PaneStatuses::default();
        statuses
            .set(&["terminal_2", "retained"], None, &manifest())
            .unwrap();
        assert!(statuses.snapshot(&manifest(), &[]).tabs.is_empty());
        let tabs = [tab(42, 0, false)];
        assert!(statuses.snapshot(&HashMap::new(), &tabs).tabs[&42].is_empty());
        assert_eq!(
            statuses.snapshot(&manifest(), &tabs).tabs[&42][0].status,
            "retained"
        );
    }

    #[test]
    fn deterministic_numeric_order_with_terminals_before_plugins() {
        let panes = vec![
            pane(10, true),
            pane(10, false),
            pane(2, true),
            pane(2, false),
            pane(1, false),
        ];
        let mut manifest = HashMap::from([(4, panes)]);
        let mut statuses = PaneStatuses::default();
        for id in ["plugin_10", "terminal_10", "plugin_2", "terminal_2"] {
            statuses.set(&[id, "x"], None, &manifest).unwrap();
        }
        for _ in 0..5 {
            let snapshot = statuses.snapshot(&manifest, &[tab(99, 4, true)]);
            let ids: Vec<_> = snapshot.tabs[&99].iter().map(|item| item.pane_id).collect();
            assert_eq!(
                ids,
                [
                    PaneId::Terminal(2),
                    PaneId::Terminal(10),
                    PaneId::Plugin(2),
                    PaneId::Plugin(10)
                ]
            );
            manifest.get_mut(&4).unwrap().rotate_left(1);
        }
    }
}
