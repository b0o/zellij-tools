use std::collections::{BTreeMap, HashSet};

use crate::scratchpad::{ScratchpadDisplayState, ScratchpadStatusItem, ScratchpadStatusSnapshot};

const DEFAULT_FORMAT: &str = "{current_items}";
const DEFAULT_ITEM_FORMAT: &str = "{icon} {name}";
const DEFAULT_SEPARATOR: &str = " ";
const DEFAULT_VISIBLE_ICON: &str = "●";
const DEFAULT_HIDDEN_ICON: &str = "○";
const DEFAULT_CLOSED_ICON: &str = "×";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ZjstatusConfigPatch {
    outputs: BTreeMap<OutputKey, OutputConfigPatch>,
    refresh_ms: Option<u32>,
}

/// Output identity. Global names and tab fields occupy separate namespaces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OutputKey {
    Global(String),
    Tab(String),
}

/// Integration settings finalized after all configuration layers are merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZjstatusConfig {
    pub outputs: BTreeMap<OutputKey, OutputConfig>,
    pub refresh_ms: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OutputConfigPatch {
    enabled: Option<bool>,
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
    tab_item_format: Option<String>,
    tab_item_focused_format: Option<String>,
    tab_item_mru_format: Option<String>,
    tab_item_visible_format: Option<String>,
    tab_item_hidden_format: Option<String>,
    tab_item_closed_format: Option<String>,
    tab_item_separator: Option<String>,
    item_separator: Option<String>,
    current_item_separator: Option<String>,
    global_item_separator: Option<String>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
}

/// Per-output formatting and filters. The publisher checks `enabled` before rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputConfig {
    pub enabled: bool,
    tab_scoped: bool,
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
    tab_item_format: Option<String>,
    tab_item_focused_format: Option<String>,
    tab_item_mru_format: Option<String>,
    tab_item_visible_format: Option<String>,
    tab_item_hidden_format: Option<String>,
    tab_item_closed_format: Option<String>,
    tab_item_separator: Option<String>,
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
        if self.patch.outputs.is_empty() {
            return Ok(None);
        }
        Ok(Some(ZjstatusConfig {
            refresh_ms: self.patch.refresh_ms.unwrap_or(2000),
            outputs: self
                .patch
                .outputs
                .into_iter()
                .map(|(key, patch)| {
                    let config = OutputConfig::from_patch(patch, matches!(key, OutputKey::Tab(_)));
                    (key, config)
                })
                .collect(),
        }))
    }
}

impl ZjstatusConfigPatch {
    fn overlay(&mut self, other: Self) {
        overlay_option(&mut self.refresh_ms, other.refresh_ms);
        for (key, patch) in other.outputs {
            self.outputs.entry(key).or_default().overlay(patch);
        }
    }
}

impl OutputConfigPatch {
    fn overlay(&mut self, other: Self) {
        overlay_option(&mut self.enabled, other.enabled);
        overlay_option(&mut self.tab_item_format, other.tab_item_format);
        overlay_option(
            &mut self.tab_item_focused_format,
            other.tab_item_focused_format,
        );
        overlay_option(&mut self.tab_item_mru_format, other.tab_item_mru_format);
        overlay_option(
            &mut self.tab_item_visible_format,
            other.tab_item_visible_format,
        );
        overlay_option(
            &mut self.tab_item_hidden_format,
            other.tab_item_hidden_format,
        );
        overlay_option(
            &mut self.tab_item_closed_format,
            other.tab_item_closed_format,
        );
        overlay_option(&mut self.tab_item_separator, other.tab_item_separator);
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

impl OutputConfig {
    fn from_patch(patch: OutputConfigPatch, tab_scoped: bool) -> Self {
        Self {
            enabled: patch.enabled.unwrap_or(true),
            tab_scoped,
            tab_item_format: patch.tab_item_format,
            tab_item_focused_format: patch.tab_item_focused_format,
            tab_item_mru_format: patch.tab_item_mru_format,
            tab_item_visible_format: patch.tab_item_visible_format,
            tab_item_hidden_format: patch.tab_item_hidden_format,
            tab_item_closed_format: patch.tab_item_closed_format,
            tab_item_separator: patch.tab_item_separator,
            format: patch.format.unwrap_or_else(|| {
                if tab_scoped {
                    "{tab_items}"
                } else {
                    DEFAULT_FORMAT
                }
                .to_string()
            }),
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
        }
    }

    fn item_format(&self, scope: Scope, item: &ScratchpadStatusItem) -> &str {
        if scope == Scope::Tab {
            if item.is_focused {
                if let Some(format) = self.tab_item_focused_format.as_deref() {
                    return format;
                }
            }
            if item.is_mru {
                if let Some(format) = self.tab_item_mru_format.as_deref() {
                    return format;
                }
            }
        }
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
            (Scope::Tab, state) => {
                let (scoped, generic) = match state {
                    ScratchpadDisplayState::Visible => {
                        (&self.tab_item_visible_format, &self.item_visible_format)
                    }
                    ScratchpadDisplayState::Hidden => {
                        (&self.tab_item_hidden_format, &self.item_hidden_format)
                    }
                    ScratchpadDisplayState::Closed => {
                        (&self.tab_item_closed_format, &self.item_closed_format)
                    }
                };
                scoped
                    .as_deref()
                    .or(generic.as_deref())
                    .or(self.tab_item_format.as_deref())
                    .unwrap_or(&self.item_format)
            }
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
            Scope::Tab => self
                .tab_item_separator
                .as_deref()
                .unwrap_or(&self.item_separator),
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
    Tab,
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
    let mut patch = ZjstatusConfigPatch::default();
    for node in doc.nodes() {
        let kind = node.name().value();
        validate_node_entries(node)?;
        if kind == "refresh_ms" {
            if patch.refresh_ms.is_some() {
                return Err("duplicate zjstatus refresh_ms".to_string());
            }
            let value = (node.children().is_none() && node.entries().len() == 1)
                .then(|| node.entries()[0].value().as_i64())
                .flatten()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|&value| value > 0)
                .ok_or_else(|| {
                    "zjstatus refresh_ms must be an integer in 1..=4294967295".to_string()
                })?;
            patch.refresh_ms = Some(value);
            continue;
        }
        if !matches!(kind, "pipe" | "tab_pipe") {
            return Err(format!("unknown zjstatus option '{kind}'; legacy flat configuration is unsupported, use pipe \"name\" {{ ... }} or tab_pipe \"field\" {{ ... }}"));
        }
        let name = (node.entries().len() == 1)
            .then(|| node.entries()[0].value().as_string())
            .flatten()
            .ok_or_else(|| format!("zjstatus {kind} requires exactly one string name"))?;
        let key = if kind == "pipe" {
            if !is_valid_pipe_name(name) {
                return Err(format!("invalid zjstatus pipe name: '{name}'"));
            }
            OutputKey::Global(name.to_string())
        } else {
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'_')
            {
                return Err(format!(
                    "invalid zjstatus tab field '{name}'; expected [a-z0-9_]+"
                ));
            }
            OutputKey::Tab(name.to_string())
        };
        let children = node.children().ok_or_else(|| format!("zjstatus {kind} \"{name}\" requires a child block; legacy flat configuration is unsupported"))?;
        let output = parse_output_config(children, kind == "tab_pipe")
            .map_err(|err| format!("zjstatus {kind} \"{name}\": {err}"))?;
        if patch.outputs.insert(key, output).is_some() {
            return Err(format!("duplicate zjstatus {kind} \"{name}\" in one layer"));
        }
    }
    Ok(patch)
}

fn validate_node_entries(node: &kdl::KdlNode) -> Result<(), String> {
    if node.ty().is_some()
        || node
            .entries()
            .iter()
            .any(|entry| entry.name().is_some() || entry.ty().is_some())
    {
        return Err(format!(
            "zjstatus '{}' does not accept properties or type annotations",
            node.name().value()
        ));
    }
    Ok(())
}

fn parse_output_config(
    doc: &kdl::KdlDocument,
    tab_scoped: bool,
) -> Result<OutputConfigPatch, String> {
    let mut seen = HashSet::new();
    let mut enabled = None;
    for node in doc.nodes() {
        let key = node.name().value();
        validate_node_entries(node)?;
        if !seen.insert(key) {
            return Err(format!("duplicate option '{key}'"));
        }
        if node.children().is_some() {
            return Err(format!("option '{key}' does not accept a child block"));
        }
        if key == "enabled" {
            enabled = Some(
                (node.entries().len() == 1)
                    .then(|| node.entries()[0].value().as_bool())
                    .flatten()
                    .ok_or_else(|| "enabled requires exactly one KDL boolean".to_string())?,
            );
            continue;
        }
        if matches!(key, "include" | "exclude") {
            if node
                .entries()
                .iter()
                .any(|entry| entry.value().as_string().is_none())
            {
                return Err(format!("'{key}' requires string arguments"));
            }
            continue;
        }
        if !matches!(
            key,
            "format"
                | "empty_format"
                | "item_format"
                | "item_visible_format"
                | "item_hidden_format"
                | "item_closed_format"
                | "current_item_format"
                | "current_item_focused_format"
                | "current_item_mru_format"
                | "current_item_visible_format"
                | "current_item_hidden_format"
                | "current_item_closed_format"
                | "global_item_format"
                | "global_item_visible_format"
                | "global_item_hidden_format"
                | "global_item_closed_format"
                | "tab_item_format"
                | "tab_item_focused_format"
                | "tab_item_mru_format"
                | "tab_item_visible_format"
                | "tab_item_hidden_format"
                | "tab_item_closed_format"
                | "item_separator"
                | "current_item_separator"
                | "global_item_separator"
                | "tab_item_separator"
        ) {
            return Err(format!("unknown option '{key}'"));
        }
        if (tab_scoped && key.starts_with("current_")) || (!tab_scoped && key.starts_with("tab_")) {
            return Err(format!("unsupported scope in option '{key}'"));
        }
        let value = (node.entries().len() == 1)
            .then(|| node.entries()[0].value().as_string())
            .flatten()
            .ok_or_else(|| format!("'{key}' requires exactly one string"))?;
        for directive in value
            .split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(directive, _)| directive))
        {
            let Some((scope, suffix)) = directive.split_once('_') else {
                continue;
            };
            let known = matches!(
                suffix,
                "items"
                    | "configured_count"
                    | "rendered_count"
                    | "live_count"
                    | "visible_count"
                    | "hidden_count"
                    | "closed_count"
                    | "focused_name"
                    | "focused_title"
            );
            if known
                && ((tab_scoped && matches!(scope, "current" | "other"))
                    || (!tab_scoped && scope == "tab"))
            {
                return Err(format!(
                    "unsupported scope in placeholder '{{{directive}}}' in '{key}'"
                ));
            }
        }
    }
    let patch = OutputConfigPatch {
        enabled,
        tab_item_format: parse_string_child(doc, "tab_item_format"),
        tab_item_focused_format: parse_string_child(doc, "tab_item_focused_format"),
        tab_item_mru_format: parse_string_child(doc, "tab_item_mru_format"),
        tab_item_visible_format: parse_string_child(doc, "tab_item_visible_format"),
        tab_item_hidden_format: parse_string_child(doc, "tab_item_hidden_format"),
        tab_item_closed_format: parse_string_child(doc, "tab_item_closed_format"),
        tab_item_separator: parse_string_child(doc, "tab_item_separator"),
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

/// Render a global output or an explicitly supplied tab projection.
/// Missing tab items are empty, never an alias for the current tab.
pub fn render(
    config: &OutputConfig,
    snapshot: &ScratchpadStatusSnapshot,
    tab_items: Option<&[ScratchpadStatusItem]>,
) -> String {
    let global = render_scope(
        config,
        Scope::Global,
        &snapshot.global_items,
        &snapshot.global_count_items,
    );
    let mut output = BTreeMap::new();
    if config.tab_scoped {
        let items = tab_items.unwrap_or_default();
        let tab = render_scope(config, Scope::Tab, items, items);
        replace_scope_directives(&mut output, "tab", &tab);
    } else {
        let current = render_scope(
            config,
            Scope::Current,
            &snapshot.current_items,
            &snapshot.current_items,
        );
        replace_scope_directives(&mut output, "current", &current);
        replace_other_count_directives(&mut output, &current, &global);
    }
    replace_scope_directives(&mut output, "global", &global);

    let sanitize = |value: &str| {
        if config.tab_scoped {
            sanitize_tab_payload(value)
        } else {
            sanitize_pipe_payload(value)
        }
    };
    let sanitized = sanitize(&substitute(&config.format, &output));
    if sanitized.is_empty() {
        sanitize(&config.empty_format)
    } else {
        sanitized
    }
}

/// Global-pipe empty-output policy only. Tab outputs must always publish clears.
pub fn should_publish_empty(config: &OutputConfig) -> bool {
    !sanitize_pipe_payload(&config.empty_format).is_empty()
}

pub fn zjstatus_payload(pipe: &str, rendered_output: &str) -> String {
    format!("zjstatus::pipe::pipe_{pipe}::{rendered_output}")
}

/// Build a tab command, preserving embedded separators and the empty clear suffix.
pub fn zjstatus_tab_payload(tab_id: usize, field: &str, rendered_output: &str) -> String {
    format!(
        "zjstatus::tab_pipe::{tab_id}::{field}::{}",
        sanitize_tab_payload(rendered_output)
    )
}

fn render_scope<'a>(
    config: &OutputConfig,
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
    let mut filtered_render_items = render_items
        .iter()
        .filter(|item| {
            include
                .as_ref()
                .is_none_or(|names| names.contains(item.name.as_str()))
        })
        .filter(|item| !exclude.contains(item.name.as_str()))
        .collect::<Vec<_>>();
    if scope == Scope::Tab {
        filtered_render_items.sort_by(|left, right| left.name.cmp(&right.name));
    }
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

fn replace_scope_directives(
    output: &mut BTreeMap<String, String>,
    prefix: &str,
    scope: &ScopeRender<'_>,
) {
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
    output: &mut BTreeMap<String, String>,
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

fn render_item(config: &OutputConfig, scope: Scope, item: &ScratchpadStatusItem) -> String {
    let mut output = BTreeMap::new();
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
    substitute(config.item_format(scope, item), &output)
}

impl OutputConfig {
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
    sanitize_tab_payload(value).replace("::", ": :")
}

fn sanitize_tab_payload(value: &str) -> String {
    value.replace(['\r', '\n'], " ")
}

fn escape_text_value(value: &str) -> String {
    sanitize_tab_payload(value)
        .replace("#[", "# [")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

fn replace(output: &mut BTreeMap<String, String>, from: &str, to: &str) {
    output.insert(from.to_string(), to.to_string());
}

fn substitute(template: &str, values: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    // Scan the template only: inserted item text and data never become directives.
    while let Some(start) = remaining.find('{') {
        output.push_str(&remaining[..start]);
        remaining = &remaining[start..];
        let Some(end) = remaining.find('}') else {
            break;
        };
        let directive = &remaining[..=end];
        output.push_str(
            values
                .get(directive)
                .map(String::as_str)
                .unwrap_or(directive),
        );
        remaining = &remaining[end + 1..];
    }
    output.push_str(remaining);
    output
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

    fn integration(input: &str) -> ZjstatusConfig {
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(parse_zjstatus_config_kdl(input).unwrap());
        layers.into_config().unwrap().unwrap()
    }

    fn config(input: &str) -> OutputConfig {
        integration(input).outputs.into_values().next().unwrap()
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
        let config = integration(r#"pipe "scratchpads" {}"#);

        assert_eq!(config.refresh_ms, 2000);
        assert_eq!(
            config.outputs[&OutputKey::Global("scratchpads".into())].format,
            DEFAULT_FORMAT
        );
    }

    #[test]
    fn overlays_external_options_without_resetting_inline() {
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(parse_zjstatus_config_kdl(r#"pipe "scratchpads" {}"#).unwrap());
        layers.push(
            parse_zjstatus_config_kdl(r#"pipe "scratchpads" { format "{global_items}"; }"#)
                .unwrap(),
        );

        let config = layers.into_config().unwrap().unwrap();

        assert_eq!(config.outputs.len(), 1);
        assert_eq!(
            config.outputs[&OutputKey::Global("scratchpads".into())].format,
            "{global_items}"
        );
    }

    #[test]
    fn renders_counts_and_items() {
        let config = config(
            r##"
            pipe "scratchpads" {
            format "{current_items} ({current_visible_count}/{current_configured_count})"
            current_item_visible_format "#[fg=green]{name}:{pane_id}"
            current_item_hidden_format "#[fg=gray]{name}"
            current_item_closed_format ""
            }
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

        let rendered = render(&config, &snapshot, None);

        assert_eq!(rendered, "#[fg=green]term:7 #[fg=gray]notes (1/3)");
    }

    #[test]
    fn filters_include_and_exclude_before_counts() {
        let config = config(
            r#"
            pipe "scratchpads" {
            format "{current_configured_count}:{current_rendered_count}:{current_items}"
            include "term" "notes"
            exclude "notes"
            }
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

        assert_eq!(render(&config, &snapshot, None), "1:1:● term");
    }

    #[test]
    fn empty_rendered_items_are_filtered_out() {
        let config = config(
            r#"
            pipe "scratchpads" {
            format "{current_rendered_count}:{current_items}"
            current_item_visible_format "{name}"
            current_item_closed_format ""
            item_separator "|"
            }
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

        assert_eq!(render(&config, &snapshot, None), "1:term");
    }

    #[test]
    fn current_focused_format_overrides_state_format() {
        let config = config(
            r#"
            pipe "scratchpads" {
            format "{current_items}"
            current_item_focused_format "[{title}]"
            current_item_mru_format "MRU:{title}"
            current_item_visible_format "{title}"
            }
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

        assert_eq!(render(&config, &snapshot, None), "[term] notes");
    }

    #[test]
    fn current_mru_format_overrides_states_but_not_global_items() {
        let mut config = config(
            r#"
            pipe "scratchpads" {
            format "{current_rendered_count}:{current_items}|{global_items}"
            item_format "{name}"
            current_item_mru_format "MRU:{name}"
            current_item_visible_format "visible"
            current_item_hidden_format "hidden"
            current_item_closed_format ""
            }
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
            assert_eq!(render(&config, &snapshot, None), "1:MRU:term|term");
            snapshot.current_items[0].is_focused = true;
            assert_eq!(render(&config, &snapshot, None), "1:MRU:term|term");
        }

        let mut mru = item("term", ScratchpadDisplayState::Hidden);
        mru.is_mru = true;
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![mru],
            ..Default::default()
        };
        config.current_item_mru_format = Some(String::new());
        assert_eq!(render(&config, &snapshot, None), "0:|");
        config.current_item_mru_format = None;
        assert_eq!(render(&config, &snapshot, None), "1:hidden|");
    }

    #[test]
    fn overlays_mru_format_without_resetting_other_options() {
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(
            parse_zjstatus_config_kdl(
                "pipe \"scratchpads\" { current_item_mru_format \"inline\"; }",
            )
            .unwrap(),
        );
        layers.push(
            parse_zjstatus_config_kdl("pipe \"scratchpads\" { format \"{current_items}\"; }")
                .unwrap(),
        );
        assert_eq!(
            layers.patch.outputs[&OutputKey::Global("scratchpads".into())]
                .current_item_mru_format
                .as_deref(),
            Some("inline")
        );
        layers.push(
            parse_zjstatus_config_kdl(
                "pipe \"scratchpads\" { current_item_mru_format \"external\"; }",
            )
            .unwrap(),
        );
        let config = layers.into_config().unwrap().unwrap();
        assert_eq!(
            config.outputs[&OutputKey::Global("scratchpads".into())]
                .current_item_mru_format
                .as_deref(),
            Some("external")
        );
        assert_eq!(config.outputs.len(), 1);
    }

    #[test]
    fn icons_remain_fixed() {
        let config = config(
            r#"
            pipe "scratchpads" {
            item_format "{icon}"
            }
            "#,
        );
        let snapshot = ScratchpadStatusSnapshot {
            current_items: vec![item("term", ScratchpadDisplayState::Visible)],
            global_count_items: Vec::new(),
            global_items: Vec::new(),
        };

        assert_eq!(render(&config, &snapshot, None), "●");
        assert!(
            parse_zjstatus_config_kdl(r#"pipe "scratchpads" { icons { visible "V"; }; }"#).is_err()
        );
    }

    #[test]
    fn global_counts_use_instances_not_aggregate_items() {
        let config = config(
            r#"
            pipe "scratchpads" {
            format "{global_items}:{global_live_count}:{global_visible_count}:{global_rendered_count}"
            global_item_visible_format "{name}"
            global_item_closed_format ""
            item_separator "|"
            }
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

        assert_eq!(render(&config, &snapshot, None), "term:2:2:2");
    }

    #[test]
    fn other_counts_are_global_minus_current() {
        let config = config(
            r#"
            pipe "scratchpads" {
            format "{other_configured_count}:{other_rendered_count}:{other_live_count}:{other_visible_count}:{other_hidden_count}:{other_closed_count}"
            current_item_visible_format "{name}"
            current_item_hidden_format "{name}"
            current_item_closed_format ""
            global_item_visible_format "{name}"
            global_item_hidden_format "{name}"
            global_item_closed_format ""
            }
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

        assert_eq!(render(&config, &snapshot, None), "1:2:2:1:1:0");
    }

    #[test]
    fn text_values_cannot_inject_protocol_or_formatting() {
        let config = config(
            r#"
            pipe "scratchpads" {
            item_format "{name} {title}"
            }
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

        assert_eq!(
            render(&config, &snapshot, None),
            "bad: :name # [fg=red]oops next"
        );
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
        assert!(parse_zjstatus_config_kdl(r#"pipe "bad::name" {}"#).is_err());
    }

    #[test]
    fn empty_publish_requires_non_empty_empty_format() {
        let default_empty = config(r#"pipe "scratchpads" {}"#);
        let explicit_empty = config(
            r##"
            pipe "scratchpads" {
            empty_format "#[fg=gray]none"
            }
            "##,
        );

        assert!(!should_publish_empty(&default_empty));
        assert!(should_publish_empty(&explicit_empty));
    }

    #[test]
    fn multiple_outputs_have_independent_defaults_and_deterministic_keys() {
        let config = integration(
            r#"
            tab_pipe "scratchpads" {}
            pipe "Z-9" { item_format "custom"; }
            pipe "scratchpads" {}
            tab_pipe "count_2" { format "{tab_live_count}"; }
        "#,
        );
        assert_eq!(
            config.outputs.keys().cloned().collect::<Vec<_>>(),
            vec![
                OutputKey::Global("Z-9".into()),
                OutputKey::Global("scratchpads".into()),
                OutputKey::Tab("count_2".into()),
                OutputKey::Tab("scratchpads".into()),
            ]
        );
        let global = &config.outputs[&OutputKey::Global("scratchpads".into())];
        let tab = &config.outputs[&OutputKey::Tab("scratchpads".into())];
        assert_eq!(global.format, "{current_items}");
        assert_eq!(tab.format, "{tab_items}");
        assert_eq!(tab.item_format, DEFAULT_ITEM_FORMAT);
        assert_eq!(global.item_format, DEFAULT_ITEM_FORMAT);
        assert!(config.outputs.values().all(|output| output.enabled));
        assert!(ZjstatusConfigLayers::default()
            .into_config()
            .unwrap()
            .is_none());
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(parse_zjstatus_config_kdl("refresh_ms 17").unwrap());
        assert!(layers.into_config().unwrap().is_none());
    }

    #[test]
    fn layers_merge_by_identity_preserving_empty_values_and_disabled_entries() {
        let key = OutputKey::Tab("scratchpads".into());
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(
            parse_zjstatus_config_kdl(
                r#"
            refresh_ms 100
            pipe "scratchpads" { item_format "global"; }
            tab_pipe "scratchpads" {
                format "inline"
                empty_format "fallback"
                include "term"
                exclude "notes"
                tab_item_format "{name}"
                tab_item_focused_format "focus"
                tab_item_mru_format "mru"
                tab_item_visible_format "visible"
                tab_item_hidden_format "hidden"
                tab_item_closed_format "closed"
                tab_item_separator "|"
            }
        "#,
            )
            .unwrap(),
        );
        layers.push(
            parse_zjstatus_config_kdl(
                r#"
            refresh_ms 300
            tab_pipe "scratchpads" {
                enabled false
                format ""
                empty_format ""
                include
                exclude
                tab_item_focused_format ""
                tab_item_hidden_format ""
                tab_item_separator ""
            }
        "#,
            )
            .unwrap(),
        );
        let merged = layers.clone().into_config().unwrap().unwrap();
        assert_eq!(merged.refresh_ms, 300);
        assert_eq!(merged.outputs.len(), 2);
        let output = &merged.outputs[&key];
        assert!(!output.enabled);
        assert_eq!(output.format, "");
        assert_eq!(output.empty_format, "");
        assert!(output.include.is_empty());
        assert!(output.exclude.is_empty());
        assert_eq!(output.tab_item_format.as_deref(), Some("{name}"));
        assert_eq!(output.tab_item_focused_format.as_deref(), Some(""));
        assert_eq!(output.tab_item_mru_format.as_deref(), Some("mru"));
        assert_eq!(output.tab_item_visible_format.as_deref(), Some("visible"));
        assert_eq!(output.tab_item_hidden_format.as_deref(), Some(""));
        assert_eq!(output.tab_item_closed_format.as_deref(), Some("closed"));
        assert_eq!(output.tab_item_separator.as_deref(), Some(""));
        layers.push(
            parse_zjstatus_config_kdl(
                r#"
            tab_pipe "scratchpads" { enabled true; include "notes" "git"; }
        "#,
            )
            .unwrap(),
        );
        let merged = layers.into_config().unwrap().unwrap();
        assert!(merged.outputs[&key].enabled);
        assert_eq!(merged.outputs[&key].include, ["notes", "git"]);
        assert_eq!(merged.outputs[&key].format, "");
        assert_eq!(
            merged.outputs[&OutputKey::Global("scratchpads".into())].item_format,
            "global"
        );
    }

    #[test]
    fn rejects_legacy_duplicates_and_invalid_structure_or_types() {
        for input in [
            r#"pipe "scratchpads""#,
            r#"format "{current_items}""#,
            "pipe \"old\"\ntab_pipe \"new\" {}",
            "pipe \"same\" {}\npipe \"same\" {}",
            "tab_pipe \"same\" {}\ntab_pipe \"same\" {}",
            "refresh_ms 1\nrefresh_ms 2",
            "refresh_ms 1 {}",
            "unknown {}",
            "pipe {}",
            "pipe 1 {}",
            r#"pipe "one" "two" {}"#,
            r#"pipe name="one" {}"#,
            r#"(kind)pipe "one" {}"#,
            r#"pipe (kind)"one" {}"#,
            r#"pipe "one" { enabled "false"; }"#,
            r#"pipe "one" { enabled true false; }"#,
            r#"pipe "one" { enabled; }"#,
            r#"pipe "one" { format; }"#,
            r#"pipe "one" { format 1; }"#,
            r#"pipe "one" { format "a" "b"; }"#,
            r#"pipe "one" { format value="a"; }"#,
            r#"pipe "one" { format (kind)"a"; }"#,
            r#"pipe "one" { format "a" {}; }"#,
            r#"pipe "one" { format "a"; format "b"; }"#,
            r#"pipe "one" { include "a" 2; }"#,
            r#"pipe "one" { exclude false; }"#,
            r#"pipe "one" { typo "value"; }"#,
            r#"pipe "one" { refresh_ms 10; }"#,
        ] {
            assert!(
                parse_zjstatus_config_kdl(input).is_err(),
                "accepted {input}"
            );
        }
        for name in ["", "Upper", "has-hyphen", "has space", "bad::name", "é"] {
            assert!(
                parse_zjstatus_config_kdl(&format!("tab_pipe {name:?} {{}}")).is_err(),
                "accepted {name}"
            );
        }
        for value in [
            "0",
            "-1",
            "4294967296",
            "1.5",
            "true",
            "null",
            "\"2000\"",
            "",
            "1 2",
        ] {
            let error = parse_zjstatus_config_kdl(&format!("refresh_ms {value}")).unwrap_err();
            assert!(error.contains("refresh_ms"), "{error}");
        }
        for value in [1, u32::MAX] {
            let config = integration(&format!("refresh_ms {value}\ntab_pipe \"x\" {{}}"));
            assert_eq!(config.refresh_ms, value);
        }
    }

    #[test]
    fn rejects_unsupported_known_scopes() {
        for suffix in [
            "items",
            "configured_count",
            "rendered_count",
            "live_count",
            "visible_count",
            "hidden_count",
            "closed_count",
            "focused_name",
            "focused_title",
        ] {
            for (kind, scope) in [
                ("pipe", "tab"),
                ("tab_pipe", "current"),
                ("tab_pipe", "other"),
            ] {
                let input = format!("{kind} \"x\" {{ format \"{{{scope}_{suffix}}}\"; }}");
                let error = parse_zjstatus_config_kdl(&input).unwrap_err();
                assert!(error.contains("unsupported scope"), "{error}");
            }
        }
        for (kind, scope) in [("pipe", "tab"), ("tab_pipe", "current")] {
            for suffix in [
                "format",
                "focused_format",
                "mru_format",
                "visible_format",
                "hidden_format",
                "closed_format",
                "separator",
            ] {
                let input = format!("{kind} \"x\" {{ {scope}_item_{suffix} \"\"; }}");
                assert!(parse_zjstatus_config_kdl(&input).is_err(), "{input}");
            }
        }
    }

    #[test]
    fn tab_scopes_are_explicit_and_global_counts_still_count_instances() {
        let global = config(
            r#"pipe "x" { format "{global_items}:{global_live_count}"; item_format "{name}"; }"#,
        );
        let config = config(
            r#"tab_pipe "x" {
            format "{tab_items}|{tab_configured_count}/{tab_rendered_count}/{tab_live_count}/{tab_visible_count}/{tab_hidden_count}/{tab_closed_count}|{tab_focused_name}:{tab_focused_title}|{global_items}:{global_live_count}"
            item_format "{name}:{state}:{pane_id}:{tab_id}:{tab_position}:{is_focused}"
            global_item_format "{name}"
        }"#,
        );
        let mut snapshot = ScratchpadStatusSnapshot {
            current_items: vec![item("current", ScratchpadDisplayState::Visible)],
            global_items: vec![item("term", ScratchpadDisplayState::Visible)],
            global_count_items: vec![item("term", ScratchpadDisplayState::Visible); 2],
        };
        for (state, counts, pane, focused) in [
            (ScratchpadDisplayState::Visible, "1/1/1/1/0/0", "7", true),
            (ScratchpadDisplayState::Hidden, "1/1/1/0/1/0", "7", false),
            (ScratchpadDisplayState::Closed, "1/1/0/0/0/1", "", false),
        ] {
            let mut local = item("term", state);
            local.title = "Terminal".into();
            local.is_focused = focused;
            if state == ScratchpadDisplayState::Closed {
                local.pane_id = None;
            }
            let items = [local];
            let expected = format!(
                "term:{}:{pane}:11:2:{focused}|{counts}|{}|term:2",
                state.as_str(),
                if focused { "term:Terminal" } else { ":" }
            );
            assert_eq!(render(&config, &snapshot, Some(&items)), expected);
            snapshot.current_items.clear();
            assert_eq!(render(&config, &snapshot, Some(&items)), expected);
        }
        assert_eq!(render(&config, &snapshot, None), "|0/0/0/0/0/0|:|term:2");
        assert_eq!(
            render(
                &global,
                &snapshot,
                Some(&[item("tab", ScratchpadDisplayState::Hidden)])
            ),
            "term:2"
        );
    }

    #[test]
    fn tab_filtering_counts_and_separators_use_rendered_items_only() {
        let config = config(
            r#"tab_pipe "x" {
            format "{tab_items}|{tab_configured_count}/{tab_rendered_count}/{tab_live_count}/{tab_closed_count}"
            include "z" "closed" "a" "skip"
            exclude "skip"
            item_format "{name}"
            item_closed_format ""
            item_separator "wrong"
            tab_item_separator ","
        }"#,
        );
        let items = [
            item("z", ScratchpadDisplayState::Hidden),
            item("closed", ScratchpadDisplayState::Closed),
            item("a", ScratchpadDisplayState::Visible),
            item("skip", ScratchpadDisplayState::Visible),
            item("not_included", ScratchpadDisplayState::Visible),
        ];
        assert_eq!(
            render(&config, &ScratchpadStatusSnapshot::default(), Some(&items)),
            "a,z|3/2/2/1"
        );
    }

    #[test]
    fn tab_format_precedence_includes_meaningful_empty_overrides() {
        let config = config(
            r#"tab_pipe "x" {
            item_format "generic"
            tab_item_format "scoped"
            item_visible_format "generic state"
            item_hidden_format "generic state"
            item_closed_format "generic state"
            tab_item_visible_format "scoped state"
            tab_item_hidden_format "scoped state"
            tab_item_closed_format "scoped state"
            tab_item_mru_format "mru"
            tab_item_focused_format "focus"
        }"#,
        );
        for state in [
            ScratchpadDisplayState::Visible,
            ScratchpadDisplayState::Hidden,
            ScratchpadDisplayState::Closed,
        ] {
            let mut config = config.clone();
            let mut item = item("term", state);
            item.is_focused = true;
            item.is_mru = true;
            assert_eq!(render_item(&config, Scope::Tab, &item), "focus");
            config.tab_item_focused_format = Some(String::new());
            assert_eq!(render_item(&config, Scope::Tab, &item), "");
            config.tab_item_focused_format = None;
            assert_eq!(render_item(&config, Scope::Tab, &item), "mru");
            config.tab_item_mru_format = Some(String::new());
            assert_eq!(render_item(&config, Scope::Tab, &item), "");
            config.tab_item_mru_format = None;
            assert_eq!(render_item(&config, Scope::Tab, &item), "scoped state");
            config.tab_item_visible_format = None;
            config.tab_item_hidden_format = None;
            config.tab_item_closed_format = None;
            assert_eq!(render_item(&config, Scope::Tab, &item), "generic state");
            config.item_visible_format = None;
            config.item_hidden_format = None;
            config.item_closed_format = None;
            assert_eq!(render_item(&config, Scope::Tab, &item), "scoped");
            config.tab_item_format = None;
            assert_eq!(render_item(&config, Scope::Tab, &item), "generic");
        }
    }

    #[test]
    fn substitutions_are_nonrecursive_and_protocol_encoding_is_destination_specific() {
        for kind in ["pipe", "tab_pipe"] {
            let scope = if kind == "pipe" { "current" } else { "tab" };
            let config = config(&format!(
                r##"{kind} "x" {{
                format "#[bold]{{{scope}_items}}|{{{scope}_focused_name}}|{{{scope}_focused_title}}|{{global_live_count}}|{{unknown}}"
                item_format "{{name}}/{{title}}/{{state}}"
            }}"##
            ));
            let mut item = item(
                "{title}::{global_live_count}",
                ScratchpadDisplayState::Hidden,
            );
            item.title = "#[fg=red]{state}\r\nnext".into();
            item.is_focused = true;
            let snapshot = ScratchpadStatusSnapshot {
                current_items: vec![item.clone()],
                ..Default::default()
            };
            let rendered = render(&config, &snapshot, Some(&[item]));
            let separator = if kind == "pipe" { ": :" } else { "::" };
            assert_eq!(rendered, format!("#[bold]\\{{title\\}}{separator}\\{{global_live_count\\}}/# [fg=red]\\{{state\\}}  next/hidden|\\{{title\\}}{separator}\\{{global_live_count\\}}|# [fg=red]\\{{state\\}}  next|0|{{unknown}}"));
        }
        let config = config(r#"tab_pipe "x" { item_format "{global_live_count}"; }"#);
        assert_eq!(
            render(
                &config,
                &ScratchpadStatusSnapshot::default(),
                Some(&[item("term", ScratchpadDisplayState::Visible)])
            ),
            "{global_live_count}"
        );
        assert_eq!(
            zjstatus_tab_payload(42, "x", "a::b\r\nc"),
            "zjstatus::tab_pipe::42::x::a::b  c"
        );
        assert_eq!(
            zjstatus_tab_payload(0, "x", ""),
            "zjstatus::tab_pipe::0::x::"
        );
        assert_eq!(
            zjstatus_tab_payload(42, "x", " "),
            "zjstatus::tab_pipe::42::x:: "
        );
    }

    #[test]
    fn empty_fallback_and_count_only_outputs_have_no_implicit_hiding() {
        let snapshot = ScratchpadStatusSnapshot::default();
        for kind in ["pipe", "tab_pipe"] {
            let scope = if kind == "pipe" { "current" } else { "tab" };
            for (body, expected) in [
                (String::new(), ""),
                ("empty_format \"fallback\";".into(), "fallback"),
                ("format \"\"; empty_format \"\";".into(), ""),
                ("format \" \"; empty_format \"fallback\";".into(), " "),
                (
                    format!("format \"{{{scope}_live_count}}\"; empty_format \"fallback\";"),
                    "0",
                ),
            ] {
                let config = config(&format!("{kind} \"x\" {{ {body} }}"));
                assert_eq!(render(&config, &snapshot, Some(&[])), expected);
            }
        }
    }
}
