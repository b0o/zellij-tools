use std::collections::HashSet;

use crate::scratchpad::{ScratchpadDisplayState, ScratchpadStatusItem, ScratchpadStatusSnapshot};

const DEFAULT_FORMAT: &str = "{current_items}";
const DEFAULT_ITEM_FORMAT: &str = "{icon} {name}";
const DEFAULT_SEPARATOR: &str = " ";
const DEFAULT_VISIBLE_ICON: &str = "●";
const DEFAULT_HIDDEN_ICON: &str = "○";
const DEFAULT_CLOSED_ICON: &str = "×";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ZjstatusConfigPatch {
    pipe: Option<String>,
    format: Option<String>,
    empty_format: Option<String>,
    item_format: Option<String>,
    item_visible_format: Option<String>,
    item_hidden_format: Option<String>,
    item_closed_format: Option<String>,
    current_item_format: Option<String>,
    current_item_focused_format: Option<String>,
    current_item_mru_format: Option<String>,
    current_item_visible_format: Option<String>,
    current_item_hidden_format: Option<String>,
    current_item_closed_format: Option<String>,
    global_item_format: Option<String>,
    global_item_visible_format: Option<String>,
    global_item_hidden_format: Option<String>,
    global_item_closed_format: Option<String>,
    item_separator: Option<String>,
    current_item_separator: Option<String>,
    global_item_separator: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZjstatusConfig {
    pub pipe: String,
    format: String,
    empty_format: String,
    item_format: String,
    item_visible_format: Option<String>,
    item_hidden_format: Option<String>,
    item_closed_format: Option<String>,
    current_item_format: Option<String>,
    current_item_focused_format: Option<String>,
    current_item_mru_format: Option<String>,
    current_item_visible_format: Option<String>,
    current_item_hidden_format: Option<String>,
    current_item_closed_format: Option<String>,
    global_item_format: Option<String>,
    global_item_visible_format: Option<String>,
    global_item_hidden_format: Option<String>,
    global_item_closed_format: Option<String>,
    item_separator: String,
    current_item_separator: Option<String>,
    global_item_separator: Option<String>,
    include: Vec<String>,
    exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ZjstatusConfigLayers {
    patch: ZjstatusConfigPatch,
}

impl ZjstatusConfigLayers {
    pub fn push(&mut self, patch: ZjstatusConfigPatch) {
        self.patch.overlay(patch);
    }

    pub fn into_config(self) -> Result<Option<ZjstatusConfig>, String> {
        ZjstatusConfig::from_patch(self.patch)
    }
}

impl ZjstatusConfigPatch {
    fn overlay(&mut self, other: Self) {
        overlay_option(&mut self.pipe, other.pipe);
        overlay_option(&mut self.format, other.format);
        overlay_option(&mut self.empty_format, other.empty_format);
        overlay_option(&mut self.item_format, other.item_format);
        overlay_option(&mut self.item_visible_format, other.item_visible_format);
        overlay_option(&mut self.item_hidden_format, other.item_hidden_format);
        overlay_option(&mut self.item_closed_format, other.item_closed_format);
        overlay_option(&mut self.current_item_format, other.current_item_format);
        overlay_option(
            &mut self.current_item_focused_format,
            other.current_item_focused_format,
        );
        overlay_option(
            &mut self.current_item_mru_format,
            other.current_item_mru_format,
        );
        overlay_option(
            &mut self.current_item_visible_format,
            other.current_item_visible_format,
        );
        overlay_option(
            &mut self.current_item_hidden_format,
            other.current_item_hidden_format,
        );
        overlay_option(
            &mut self.current_item_closed_format,
            other.current_item_closed_format,
        );
        overlay_option(&mut self.global_item_format, other.global_item_format);
        overlay_option(
            &mut self.global_item_visible_format,
            other.global_item_visible_format,
        );
        overlay_option(
            &mut self.global_item_hidden_format,
            other.global_item_hidden_format,
        );
        overlay_option(
            &mut self.global_item_closed_format,
            other.global_item_closed_format,
        );
        overlay_option(&mut self.item_separator, other.item_separator);
        overlay_option(
            &mut self.current_item_separator,
            other.current_item_separator,
        );
        overlay_option(&mut self.global_item_separator, other.global_item_separator);
        overlay_option(&mut self.include, other.include);
        overlay_option(&mut self.exclude, other.exclude);
    }
}

impl ZjstatusConfig {
    fn from_patch(patch: ZjstatusConfigPatch) -> Result<Option<Self>, String> {
        let Some(pipe) = patch.pipe else {
            return Ok(None);
        };
        if !is_valid_pipe_name(&pipe) {
            return Err(format!("invalid zjstatus pipe name: '{pipe}'"));
        }

        Ok(Some(Self {
            pipe,
            format: patch.format.unwrap_or_else(|| DEFAULT_FORMAT.to_string()),
            empty_format: patch.empty_format.unwrap_or_default(),
            item_format: patch
                .item_format
                .unwrap_or_else(|| DEFAULT_ITEM_FORMAT.to_string()),
            item_visible_format: patch.item_visible_format,
            item_hidden_format: patch.item_hidden_format,
            item_closed_format: patch.item_closed_format,
            current_item_format: patch.current_item_format,
            current_item_focused_format: patch.current_item_focused_format,
            current_item_mru_format: patch.current_item_mru_format,
            current_item_visible_format: patch.current_item_visible_format,
            current_item_hidden_format: patch.current_item_hidden_format,
            current_item_closed_format: patch.current_item_closed_format,
            global_item_format: patch.global_item_format,
            global_item_visible_format: patch.global_item_visible_format,
            global_item_hidden_format: patch.global_item_hidden_format,
            global_item_closed_format: patch.global_item_closed_format,
            item_separator: patch
                .item_separator
                .unwrap_or_else(|| DEFAULT_SEPARATOR.to_string()),
            current_item_separator: patch.current_item_separator,
            global_item_separator: patch.global_item_separator,
            include: patch.include.unwrap_or_default(),
            exclude: patch.exclude.unwrap_or_default(),
        }))
    }

    fn item_format(&self, scope: Scope, item: &ScratchpadStatusItem) -> &str {
        if scope == Scope::Current && item.is_focused {
            if let Some(format) = self.current_item_focused_format.as_deref() {
                return format;
            }
        }

        if scope == Scope::Current && item.is_mru {
            if let Some(format) = self.current_item_mru_format.as_deref() {
                return format;
            }
        }

        match (scope, item.state) {
            (Scope::Current, ScratchpadDisplayState::Visible) => self
                .current_item_visible_format
                .as_deref()
                .or(self.item_visible_format.as_deref())
                .or(self.current_item_format.as_deref())
                .unwrap_or(&self.item_format),
            (Scope::Current, ScratchpadDisplayState::Hidden) => self
                .current_item_hidden_format
                .as_deref()
                .or(self.item_hidden_format.as_deref())
                .or(self.current_item_format.as_deref())
                .unwrap_or(&self.item_format),
            (Scope::Current, ScratchpadDisplayState::Closed) => self
                .current_item_closed_format
                .as_deref()
                .or(self.item_closed_format.as_deref())
                .or(self.current_item_format.as_deref())
                .unwrap_or(&self.item_format),
            (Scope::Global, ScratchpadDisplayState::Visible) => self
                .global_item_visible_format
                .as_deref()
                .or(self.item_visible_format.as_deref())
                .or(self.global_item_format.as_deref())
                .unwrap_or(&self.item_format),
            (Scope::Global, ScratchpadDisplayState::Hidden) => self
                .global_item_hidden_format
                .as_deref()
                .or(self.item_hidden_format.as_deref())
                .or(self.global_item_format.as_deref())
                .unwrap_or(&self.item_format),
            (Scope::Global, ScratchpadDisplayState::Closed) => self
                .global_item_closed_format
                .as_deref()
                .or(self.item_closed_format.as_deref())
                .or(self.global_item_format.as_deref())
                .unwrap_or(&self.item_format),
        }
    }

    fn item_separator(&self, scope: Scope) -> &str {
        match scope {
            Scope::Current => self
                .current_item_separator
                .as_deref()
                .unwrap_or(&self.item_separator),
            Scope::Global => self
                .global_item_separator
                .as_deref()
                .unwrap_or(&self.item_separator),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Current,
    Global,
}

struct ScopeRender<'a> {
    items: Vec<&'a ScratchpadStatusItem>,
    rendered_items: String,
    rendered_count: usize,
    focused: Option<&'a ScratchpadStatusItem>,
}

pub fn parse_zjstatus_config_kdl(input: &str) -> Result<ZjstatusConfigPatch, String> {
    let doc: kdl::KdlDocument = input
        .parse()
        .map_err(|err| format!("KDL parse error: {err}"))?;
    parse_zjstatus_config_doc(&doc)
}

pub fn parse_zjstatus_config_doc(doc: &kdl::KdlDocument) -> Result<ZjstatusConfigPatch, String> {
    let patch = ZjstatusConfigPatch {
        pipe: parse_string_child(doc, "pipe"),
        format: parse_string_child(doc, "format"),
        empty_format: parse_string_child(doc, "empty_format"),
        item_format: parse_string_child(doc, "item_format"),
        item_visible_format: parse_string_child(doc, "item_visible_format"),
        item_hidden_format: parse_string_child(doc, "item_hidden_format"),
        item_closed_format: parse_string_child(doc, "item_closed_format"),
        current_item_format: parse_string_child(doc, "current_item_format"),
        current_item_visible_format: parse_string_child(doc, "current_item_visible_format"),
        current_item_hidden_format: parse_string_child(doc, "current_item_hidden_format"),
        current_item_closed_format: parse_string_child(doc, "current_item_closed_format"),
        global_item_format: parse_string_child(doc, "global_item_format"),
        global_item_visible_format: parse_string_child(doc, "global_item_visible_format"),
        global_item_hidden_format: parse_string_child(doc, "global_item_hidden_format"),
        global_item_closed_format: parse_string_child(doc, "global_item_closed_format"),
        item_separator: parse_string_child(doc, "item_separator"),
        current_item_separator: parse_string_child(doc, "current_item_separator"),
        global_item_separator: parse_string_child(doc, "global_item_separator"),
        current_item_focused_format: parse_string_child(doc, "current_item_focused_format"),
        current_item_mru_format: parse_string_child(doc, "current_item_mru_format"),
        include: parse_string_args_child(doc, "include"),
        exclude: parse_string_args_child(doc, "exclude"),
    };

    Ok(patch)
}

pub fn render(config: &ZjstatusConfig, snapshot: &ScratchpadStatusSnapshot) -> String {
    let current = render_scope(
        config,
        Scope::Current,
        &snapshot.current_items,
        &snapshot.current_items,
    );
    let global = render_scope(
        config,
        Scope::Global,
        &snapshot.global_items,
        &snapshot.global_count_items,
    );
    let mut output = config.format.clone();

    replace_scope_directives(&mut output, "current", &current);
    replace_scope_directives(&mut output, "global", &global);
    replace_other_count_directives(&mut output, &current, &global);

    let sanitized = sanitize_pipe_payload(&output);
    if sanitized.is_empty() {
        sanitize_pipe_payload(&config.empty_format)
    } else {
        sanitized
    }
}

pub fn should_publish_empty(config: &ZjstatusConfig) -> bool {
    !sanitize_pipe_payload(&config.empty_format).is_empty()
}

pub fn zjstatus_payload(pipe: &str, rendered_output: &str) -> String {
    format!("zjstatus::pipe::pipe_{pipe}::{rendered_output}")
}

fn render_scope<'a>(
    config: &ZjstatusConfig,
    scope: Scope,
    render_items: &'a [ScratchpadStatusItem],
    count_items: &'a [ScratchpadStatusItem],
) -> ScopeRender<'a> {
    let include = (!config.include.is_empty()).then(|| {
        config
            .include
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>()
    });
    let exclude = config
        .exclude
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let filtered_render_items = render_items
        .iter()
        .filter(|item| {
            include
                .as_ref()
                .is_none_or(|names| names.contains(item.name.as_str()))
        })
        .filter(|item| !exclude.contains(item.name.as_str()))
        .collect::<Vec<_>>();
    let filtered_count_items = count_items
        .iter()
        .filter(|item| {
            include
                .as_ref()
                .is_none_or(|names| names.contains(item.name.as_str()))
        })
        .filter(|item| !exclude.contains(item.name.as_str()))
        .collect::<Vec<_>>();
    let rendered = filtered_render_items
        .iter()
        .copied()
        .filter_map(|item| {
            let rendered = render_item(config, scope, item);
            (!rendered.is_empty()).then_some(rendered)
        })
        .collect::<Vec<_>>();
    let rendered_count = filtered_count_items
        .iter()
        .copied()
        .filter(|item| !render_item(config, scope, item).is_empty())
        .count();
    let rendered_items = rendered.join(config.item_separator(scope));
    let focused = filtered_render_items
        .iter()
        .copied()
        .find(|item| item.is_focused);

    ScopeRender {
        items: filtered_count_items,
        rendered_items,
        rendered_count,
        focused,
    }
}

fn replace_scope_directives(output: &mut String, prefix: &str, scope: &ScopeRender<'_>) {
    replace(
        output,
        &format!("{{{prefix}_items}}"),
        &scope.rendered_items,
    );
    replace(
        output,
        &format!("{{{prefix}_configured_count}}"),
        &scope.items.len().to_string(),
    );
    replace(
        output,
        &format!("{{{prefix}_rendered_count}}"),
        &scope.rendered_count().to_string(),
    );
    replace(
        output,
        &format!("{{{prefix}_live_count}}"),
        &scope.live_count().to_string(),
    );
    replace(
        output,
        &format!("{{{prefix}_visible_count}}"),
        &scope
            .state_count(ScratchpadDisplayState::Visible)
            .to_string(),
    );
    replace(
        output,
        &format!("{{{prefix}_hidden_count}}"),
        &scope
            .state_count(ScratchpadDisplayState::Hidden)
            .to_string(),
    );
    replace(
        output,
        &format!("{{{prefix}_closed_count}}"),
        &scope
            .state_count(ScratchpadDisplayState::Closed)
            .to_string(),
    );
    replace(
        output,
        &format!("{{{prefix}_focused_name}}"),
        &scope
            .focused
            .map(|item| escape_text_value(&item.name))
            .unwrap_or_default(),
    );
    replace(
        output,
        &format!("{{{prefix}_focused_title}}"),
        &scope
            .focused
            .map(|item| escape_text_value(&item.title))
            .unwrap_or_default(),
    );
}

fn replace_other_count_directives(
    output: &mut String,
    current: &ScopeRender<'_>,
    global: &ScopeRender<'_>,
) {
    replace(
        output,
        "{other_configured_count}",
        &other_count(global.configured_count(), current.configured_count()).to_string(),
    );
    replace(
        output,
        "{other_rendered_count}",
        &other_count(global.rendered_count(), current.rendered_count()).to_string(),
    );
    replace(
        output,
        "{other_live_count}",
        &other_count(global.live_count(), current.live_count()).to_string(),
    );
    replace(
        output,
        "{other_visible_count}",
        &other_count(
            global.state_count(ScratchpadDisplayState::Visible),
            current.state_count(ScratchpadDisplayState::Visible),
        )
        .to_string(),
    );
    replace(
        output,
        "{other_hidden_count}",
        &other_count(
            global.state_count(ScratchpadDisplayState::Hidden),
            current.state_count(ScratchpadDisplayState::Hidden),
        )
        .to_string(),
    );
    replace(
        output,
        "{other_closed_count}",
        &other_count(
            global.state_count(ScratchpadDisplayState::Closed),
            current.state_count(ScratchpadDisplayState::Closed),
        )
        .to_string(),
    );
}

impl ScopeRender<'_> {
    fn configured_count(&self) -> usize {
        self.items.len()
    }

    fn rendered_count(&self) -> usize {
        self.rendered_count
    }

    fn live_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state != ScratchpadDisplayState::Closed)
            .count()
    }

    fn state_count(&self, state: ScratchpadDisplayState) -> usize {
        self.items.iter().filter(|item| item.state == state).count()
    }
}

fn other_count(global: usize, current: usize) -> usize {
    global.saturating_sub(current)
}

fn render_item(config: &ZjstatusConfig, scope: Scope, item: &ScratchpadStatusItem) -> String {
    let mut output = config.item_format(scope, item).to_string();
    replace(&mut output, "{name}", &escape_text_value(&item.name));
    replace(&mut output, "{title}", &escape_text_value(&item.title));
    replace(&mut output, "{state}", item.state.as_str());
    replace(&mut output, "{icon}", config.icon(item));
    replace(
        &mut output,
        "{pane_id}",
        &item.pane_id.map(|id| id.to_string()).unwrap_or_default(),
    );
    replace(
        &mut output,
        "{tab_id}",
        &item.tab_id.map(|id| id.to_string()).unwrap_or_default(),
    );
    replace(
        &mut output,
        "{tab_position}",
        &item
            .tab_position
            .map(|position| position.to_string())
            .unwrap_or_default(),
    );
    replace(
        &mut output,
        "{is_focused}",
        if item.is_focused { "true" } else { "false" },
    );
    output
}

impl ZjstatusConfig {
    fn icon(&self, item: &ScratchpadStatusItem) -> &str {
        match item.state {
            ScratchpadDisplayState::Visible => DEFAULT_VISIBLE_ICON,
            ScratchpadDisplayState::Hidden => DEFAULT_HIDDEN_ICON,
            ScratchpadDisplayState::Closed => DEFAULT_CLOSED_ICON,
        }
    }
}

fn parse_string_child(doc: &kdl::KdlDocument, key: &str) -> Option<String> {
    doc.get(key)?
        .entries()
        .first()?
        .value()
        .as_string()
        .map(ToString::to_string)
}

fn parse_string_args_child(doc: &kdl::KdlDocument, key: &str) -> Option<Vec<String>> {
    let args = doc
        .get(key)?
        .entries()
        .iter()
        .filter_map(|entry| entry.value().as_string().map(ToString::to_string));
    Some(args.collect())
}

fn sanitize_pipe_payload(value: &str) -> String {
    value.replace(['\r', '\n'], " ").replace("::", ": :")
}

fn escape_text_value(value: &str) -> String {
    sanitize_pipe_payload(value)
        .replace("#[", "# [")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

fn replace(output: &mut String, from: &str, to: &str) {
    if output.contains(from) {
        *output = output.replace(from, to);
    }
}

fn overlay_option<T>(target: &mut Option<T>, value: Option<T>) {
    if value.is_some() {
        *target = value;
    }
}

fn is_valid_pipe_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(input: &str) -> ZjstatusConfig {
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(parse_zjstatus_config_kdl(input).unwrap());
        layers.into_config().unwrap().unwrap()
    }

    fn item(name: &str, state: ScratchpadDisplayState) -> ScratchpadStatusItem {
        ScratchpadStatusItem {
            name: name.to_string(),
            title: name.to_string(),
            state,
            pane_id: Some(7),
            tab_id: Some(11),
            tab_position: Some(2),
            is_focused: false,
            is_mru: false,
        }
    }

    #[test]
    fn parses_minimal_config() {
        let config = config(r#"pipe "scratchpads""#);

        assert_eq!(config.pipe, "scratchpads");
        assert_eq!(config.format, DEFAULT_FORMAT);
    }

    #[test]
    fn overlays_external_options_without_resetting_inline() {
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(parse_zjstatus_config_kdl(r#"pipe "scratchpads""#).unwrap());
        layers.push(parse_zjstatus_config_kdl(r#"format "{global_items}""#).unwrap());

        let config = layers.into_config().unwrap().unwrap();

        assert_eq!(config.pipe, "scratchpads");
        assert_eq!(config.format, "{global_items}");
    }

    #[test]
    fn renders_counts_and_items() {
        let config = config(
            r##"
            pipe "scratchpads"
            format "{current_items} ({current_visible_count}/{current_configured_count})"
            current_item_visible_format "#[fg=green]{name}:{pane_id}"
            current_item_hidden_format "#[fg=gray]{name}"
            current_item_closed_format ""
            "##,
        );
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![
                item("term", ScratchpadDisplayState::Visible),
                item("notes", ScratchpadDisplayState::Hidden),
                item("btop", ScratchpadDisplayState::Closed),
            ],
            global_count_items: Vec::new(),
            global_items: Vec::new(),
        };

        let rendered = render(&config, &snapshot);

        assert_eq!(rendered, "#[fg=green]term:7 #[fg=gray]notes (1/3)");
    }

    #[test]
    fn filters_include_and_exclude_before_counts() {
        let config = config(
            r#"
            pipe "scratchpads"
            format "{current_configured_count}:{current_rendered_count}:{current_items}"
            include "term" "notes"
            exclude "notes"
            "#,
        );
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![
                item("term", ScratchpadDisplayState::Visible),
                item("notes", ScratchpadDisplayState::Visible),
                item("btop", ScratchpadDisplayState::Visible),
            ],
            global_count_items: Vec::new(),
            global_items: Vec::new(),
        };

        assert_eq!(render(&config, &snapshot), "1:1:● term");
    }

    #[test]
    fn empty_rendered_items_are_filtered_out() {
        let config = config(
            r#"
            pipe "scratchpads"
            format "{current_rendered_count}:{current_items}"
            current_item_visible_format "{name}"
            current_item_closed_format ""
            item_separator "|"
            "#,
        );
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![
                item("term", ScratchpadDisplayState::Visible),
                item("notes", ScratchpadDisplayState::Closed),
            ],
            global_count_items: Vec::new(),
            global_items: Vec::new(),
        };

        assert_eq!(render(&config, &snapshot), "1:term");
    }

    #[test]
    fn current_focused_format_overrides_state_format() {
        let config = config(
            r#"
            pipe "scratchpads"
            format "{current_items}"
            current_item_focused_format "[{title}]"
            current_item_mru_format "MRU:{title}"
            current_item_visible_format "{title}"
            "#,
        );
        let mut focused = item("term", ScratchpadDisplayState::Visible);
        focused.is_focused = true;
        focused.is_mru = true;
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![focused, item("notes", ScratchpadDisplayState::Visible)],
            global_count_items: Vec::new(),
            global_items: Vec::new(),
        };

        assert_eq!(render(&config, &snapshot), "[term] notes");
    }

    #[test]
    fn current_mru_format_overrides_states_but_not_global_items() {
        let mut config = config(
            r#"
            pipe "scratchpads"
            format "{current_rendered_count}:{current_items}|{global_items}"
            item_format "{name}"
            current_item_mru_format "MRU:{name}"
            current_item_visible_format "visible"
            current_item_hidden_format "hidden"
            current_item_closed_format ""
            "#,
        );

        for state in [
            ScratchpadDisplayState::Visible,
            ScratchpadDisplayState::Hidden,
            ScratchpadDisplayState::Closed,
        ] {
            let mut mru = item("term", state);
            mru.is_mru = true;
            let mut snapshot = ScratchpadStatusSnapshot {
                current_items: vec![mru.clone()],
                global_items: vec![mru.clone()],
                global_count_items: vec![mru],
            };
            assert_eq!(render(&config, &snapshot), "1:MRU:term|term");
            snapshot.current_items[0].is_focused = true;
            assert_eq!(render(&config, &snapshot), "1:MRU:term|term");
        }

        let mut mru = item("term", ScratchpadDisplayState::Hidden);
        mru.is_mru = true;
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![mru],
            ..Default::default()
        };
        config.current_item_mru_format = Some(String::new());
        assert_eq!(render(&config, &snapshot), "0:|");
        config.current_item_mru_format = None;
        assert_eq!(render(&config, &snapshot), "1:hidden|");
    }

    #[test]
    fn overlays_mru_format_without_resetting_other_options() {
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(
            parse_zjstatus_config_kdl("pipe \"scratchpads\"\ncurrent_item_mru_format \"inline\"")
                .unwrap(),
        );
        layers.push(parse_zjstatus_config_kdl("format \"{current_items}\"").unwrap());
        assert_eq!(
            layers.patch.current_item_mru_format.as_deref(),
            Some("inline")
        );
        layers.push(parse_zjstatus_config_kdl("current_item_mru_format \"external\"").unwrap());
        let config = layers.into_config().unwrap().unwrap();
        assert_eq!(config.current_item_mru_format.as_deref(), Some("external"));
        assert_eq!(config.pipe, "scratchpads");
    }

    #[test]
    fn icons_block_is_ignored() {
        let config = config(
            r#"
            pipe "scratchpads"
            item_format "{icon}"
            icons {
                visible "V"
                hidden "H"
                closed "C"
            }
            "#,
        );
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![item("term", ScratchpadDisplayState::Visible)],
            global_count_items: Vec::new(),
            global_items: Vec::new(),
        };

        assert_eq!(render(&config, &snapshot), "●");
    }

    #[test]
    fn global_counts_use_instances_not_aggregate_items() {
        let config = config(
            r#"
            pipe "scratchpads"
            format "{global_items}:{global_live_count}:{global_visible_count}:{global_rendered_count}"
            global_item_visible_format "{name}"
            global_item_closed_format ""
            item_separator "|"
            "#,
        );
        let mut second_term = item("term", ScratchpadDisplayState::Visible);
        second_term.pane_id = Some(8);
        second_term.tab_id = Some(12);
        second_term.tab_position = Some(3);
        let snapshot = ScratchpadStatusSnapshot {
            current_items: Vec::new(),
            global_items: vec![item("term", ScratchpadDisplayState::Visible)],
            global_count_items: vec![
                item("term", ScratchpadDisplayState::Visible),
                second_term,
                item("notes", ScratchpadDisplayState::Closed),
            ],
        };

        assert_eq!(render(&config, &snapshot), "term:2:2:2");
    }

    #[test]
    fn other_counts_are_global_minus_current() {
        let config = config(
            r#"
            pipe "scratchpads"
            format "{other_configured_count}:{other_rendered_count}:{other_live_count}:{other_visible_count}:{other_hidden_count}:{other_closed_count}"
            current_item_visible_format "{name}"
            current_item_hidden_format "{name}"
            current_item_closed_format ""
            global_item_visible_format "{name}"
            global_item_hidden_format "{name}"
            global_item_closed_format ""
            "#,
        );
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![
                item("term", ScratchpadDisplayState::Visible),
                item("notes", ScratchpadDisplayState::Closed),
            ],
            global_items: vec![
                item("term", ScratchpadDisplayState::Visible),
                item("notes", ScratchpadDisplayState::Hidden),
            ],
            global_count_items: vec![
                item("term", ScratchpadDisplayState::Visible),
                item("term", ScratchpadDisplayState::Visible),
                item("notes", ScratchpadDisplayState::Hidden),
            ],
        };

        assert_eq!(render(&config, &snapshot), "1:2:2:1:1:0");
    }

    #[test]
    fn text_values_cannot_inject_protocol_or_formatting() {
        let config = config(
            r#"
            pipe "scratchpads"
            item_format "{name} {title}"
            "#,
        );
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![ScratchpadStatusItem {
                name: "bad::name".to_string(),
                title: "#[fg=red]oops\nnext".to_string(),
                state: ScratchpadDisplayState::Visible,
                pane_id: None,
                tab_id: None,
                tab_position: None,
                is_focused: false,
                is_mru: false,
            }],
            global_count_items: Vec::new(),
            global_items: Vec::new(),
        };

        assert_eq!(render(&config, &snapshot), "bad: :name # [fg=red]oops next");
    }

    #[test]
    fn payload_uses_zjstatus_pipe_protocol() {
        assert_eq!(
            zjstatus_payload("scratchpads", "hello"),
            "zjstatus::pipe::pipe_scratchpads::hello"
        );
    }

    #[test]
    fn rejects_invalid_pipe_name() {
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(parse_zjstatus_config_kdl(r#"pipe "bad::name""#).unwrap());

        assert!(layers.into_config().is_err());
    }

    #[test]
    fn empty_publish_requires_non_empty_empty_format() {
        let default_empty = config(r#"pipe "scratchpads""#);
        let explicit_empty = config(
            r##"
            pipe "scratchpads"
            empty_format "#[fg=gray]none"
            "##,
        );

        assert!(!should_publish_empty(&default_empty));
        assert!(should_publish_empty(&explicit_empty));
    }
}
