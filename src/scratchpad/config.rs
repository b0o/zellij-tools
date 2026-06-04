use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use serde::Deserialize;
use zellij_tile::prelude::actions::Action;
use zellij_tile::prelude::{InputMode, KeyWithModifier, PaneId};

use crate::message::ParseError;

const ALL_KEYBIND_MODES: &[&str] = &[
    "normal",
    "locked",
    "resize",
    "pane",
    "move",
    "tab",
    "scroll",
    "search",
    "entersearch",
    "renametab",
    "renamepane",
    "session",
    "tmux",
];

/// Anchor point on a single axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AxisOrigin {
    /// Offset from the start (left / top). This is the default.
    Start,
    /// Centered: pane is centered, offset shifts away from center.
    Center,
    /// Offset inward from the end (right / bottom).
    End,
}

/// Reference point that x/y are calculated relative to.
///
/// ```kdl
/// origin "center"              // both axes centered
/// origin "top" "center"        // vertical=top, horizontal=center
/// origin "bottom" "right"      // anchored to bottom-right corner
/// ```
///
/// Argument order: vertical then horizontal (matches natural English:
/// "top left", "bottom center", etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Origin {
    /// Vertical anchor (top / center / bottom).
    pub vertical: AxisOrigin,
    /// Horizontal anchor (left / center / right).
    pub horizontal: AxisOrigin,
}

impl Default for Origin {
    fn default() -> Self {
        Self {
            vertical: AxisOrigin::Center,
            horizontal: AxisOrigin::Center,
        }
    }
}

/// Configuration for a scratchpad
#[derive(Debug, Clone, Deserialize)]
pub struct ScratchpadConfig {
    pub command: Vec<String>,
    /// Horizontal position: fixed columns (e.g. "10") or percent (e.g. "10%")
    pub x: Option<String>,
    /// Vertical position: fixed rows (e.g. "5") or percent (e.g. "5%")
    pub y: Option<String>,
    /// Width: fixed columns (e.g. "80") or percent (e.g. "50%")
    pub width: Option<String>,
    /// Height: fixed rows (e.g. "24") or percent (e.g. "50%")
    pub height: Option<String>,
    /// Reference point for x/y coordinates.
    pub origin: Origin,
    /// Pane title (displayed in the Zellij UI). Applied when the pane is opened.
    pub title: Option<String>,
    /// Working directory for the command.
    pub cwd: Option<String>,
    /// Client-local keybindings that trigger scratchpad actions.
    #[serde(default)]
    pub keybinds: Vec<ScratchpadKeybind>,
}

/// A parsed scratchpad keybinding, expanded to concrete input modes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ScratchpadKeybind {
    pub modes: Vec<String>,
    pub keys: Vec<String>,
    pub actions: Vec<ScratchpadKeybindAction>,
}

/// Actions supported inside scratchpad keybind blocks.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum ScratchpadKeybindAction {
    Toggle,
    Show,
    Hide,
    Close,
    SwitchToMode(String),
}

pub type ScratchpadKeybindRebind = (InputMode, KeyWithModifier, Vec<Action>);
pub type ScratchpadKeybindUnbind = (InputMode, KeyWithModifier);

/// A size value that is either a fixed number of rows/columns or a percentage.
#[derive(Debug, Clone, Copy)]
pub enum SizeValue {
    Fixed(usize),
    Percent(usize),
}

impl SizeValue {
    /// Parse a size string: plain number → Fixed, trailing '%' → Percent.
    pub fn parse(s: &str) -> Option<Self> {
        if let Some(pct) = s.strip_suffix('%') {
            pct.parse::<usize>().ok().map(SizeValue::Percent)
        } else {
            s.parse::<usize>().ok().map(SizeValue::Fixed)
        }
    }

    /// Resolve to an absolute value given the full viewport extent on this axis.
    pub fn resolve(self, viewport: usize) -> usize {
        match self {
            SizeValue::Fixed(v) => v,
            SizeValue::Percent(p) => (p as f64 / 100.0 * viewport as f64).floor() as usize,
        }
    }
}

/// Resolved floating-pane coordinates (all absolute, ready for Zellij API).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCoordinates {
    pub x: Option<String>,
    pub y: Option<String>,
    pub width: Option<String>,
    pub height: Option<String>,
}

impl ScratchpadConfig {
    /// Resolve coordinates against the current viewport, applying the origin.
    ///
    /// Returns values suitable for `FloatingPaneCoordinates::new()`.
    /// When origin is `Start` on both axes, the raw config values pass through
    /// unchanged (no viewport math needed).
    pub fn resolve_coordinates(
        &self,
        viewport_cols: usize,
        viewport_rows: usize,
    ) -> ResolvedCoordinates {
        let start_start = Origin {
            vertical: AxisOrigin::Start,
            horizontal: AxisOrigin::Start,
        };
        if self.origin == start_start {
            return ResolvedCoordinates {
                x: self.x.clone(),
                y: self.y.clone(),
                width: self.width.clone(),
                height: self.height.clone(),
            };
        }

        let pane_w = self
            .width
            .as_deref()
            .and_then(SizeValue::parse)
            .map(|s| s.resolve(viewport_cols));
        let pane_h = self
            .height
            .as_deref()
            .and_then(SizeValue::parse)
            .map(|s| s.resolve(viewport_rows));
        let offset_x = self
            .x
            .as_deref()
            .and_then(SizeValue::parse)
            .map(|s| s.resolve(viewport_cols))
            .unwrap_or(0);
        let offset_y = self
            .y
            .as_deref()
            .and_then(SizeValue::parse)
            .map(|s| s.resolve(viewport_rows))
            .unwrap_or(0);

        let resolved_x = resolve_axis(self.origin.horizontal, offset_x, pane_w, viewport_cols);
        let resolved_y = resolve_axis(self.origin.vertical, offset_y, pane_h, viewport_rows);

        ResolvedCoordinates {
            x: Some(resolved_x.to_string()),
            y: Some(resolved_y.to_string()),
            // Pass width/height through unchanged (Zellij handles them fine).
            width: self.width.clone(),
            height: self.height.clone(),
        }
    }
}

/// Resolve a single axis position given an origin, offset, optional pane size, and viewport extent.
///
/// - `Start`:  `offset`
/// - `Center`: `(viewport - pane_size) / 2 + offset`
/// - `End`:    `viewport - pane_size - offset`
///
/// When `pane_size` is `None`, it is treated as 0 (the pane will get Zellij's default size,
/// and the origin still shifts x/y accordingly).
fn resolve_axis(
    origin: AxisOrigin,
    offset: usize,
    pane_size: Option<usize>,
    viewport: usize,
) -> usize {
    let size = pane_size.unwrap_or(0);
    match origin {
        AxisOrigin::Start => offset,
        AxisOrigin::Center => {
            let center = viewport.saturating_sub(size) / 2;
            center.saturating_add(offset).min(viewport)
        }
        AxisOrigin::End => viewport.saturating_sub(size).saturating_sub(offset),
    }
}

/// Actions that can be performed on scratchpads
#[derive(Debug)]
pub enum ScratchpadAction {
    Toggle {
        name: Option<String>,
        target: ScratchpadActionTarget,
    },
    Show {
        name: String,
        target: ScratchpadActionTarget,
    },
    Hide {
        name: String,
        target: ScratchpadActionTarget,
    },
    Close {
        name: String,
        target: ScratchpadActionTarget,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ScratchpadActionTarget {
    pub tab_id: Option<usize>,
    pub source_pane: Option<PaneId>,
}

/// Check if a scratchpad name is valid (alphanumeric, underscore, hyphen)
pub fn is_valid_scratchpad_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parse a scratchpad action from message args
pub fn parse_scratchpad_action(args: &[&str]) -> Result<ScratchpadAction, ParseError> {
    let action = args.first().copied().unwrap_or("");
    let mut target = ScratchpadActionTarget::default();
    let mut action_args = Vec::new();
    let mut index = 1;
    while index < args.len() {
        match args[index] {
            "tab-id" => {
                let id = args.get(index + 1).ok_or_else(|| {
                    ParseError::InvalidArgs("tab-id requires a value".to_string())
                })?;
                target.tab_id = Some(id.parse().map_err(|_| {
                    ParseError::InvalidArgs(format!("Invalid tab ID: {}", id))
                })?);
                index += 2;
            }
            "source-pane" => {
                let pane_id = args.get(index + 1).ok_or_else(|| {
                    ParseError::InvalidArgs("source-pane requires a pane ID".to_string())
                })?;
                target.source_pane = Some(pane_id.parse().map_err(|err| {
                    ParseError::InvalidArgs(format!("Invalid source pane ID: {}", err))
                })?);
                index += 2;
            }
            arg => {
                action_args.push(arg);
                index += 1;
            }
        }
    }

    match action {
        "toggle" => {
            let name = action_args.first().map(|s| (*s).to_string());
            if let Some(ref n) = name {
                if !is_valid_scratchpad_name(n) {
                    return Err(ParseError::InvalidScratchpadName(n.clone()));
                }
            }
            Ok(ScratchpadAction::Toggle { name, target })
        }
        "show" => {
            let name = action_args.first().ok_or_else(|| {
                ParseError::InvalidArgs("show requires a scratchpad name".to_string())
            })?;
            if !is_valid_scratchpad_name(name) {
                return Err(ParseError::InvalidScratchpadName(name.to_string()));
            }
            Ok(ScratchpadAction::Show {
                name: name.to_string(),
                target,
            })
        }
        "hide" => {
            let name = action_args.first().ok_or_else(|| {
                ParseError::InvalidArgs("hide requires a scratchpad name".to_string())
            })?;
            if !is_valid_scratchpad_name(name) {
                return Err(ParseError::InvalidScratchpadName(name.to_string()));
            }
            Ok(ScratchpadAction::Hide {
                name: name.to_string(),
                target,
            })
        }
        "close" => {
            let name = action_args.first().ok_or_else(|| {
                ParseError::InvalidArgs("close requires a scratchpad name".to_string())
            })?;
            if !is_valid_scratchpad_name(name) {
                return Err(ParseError::InvalidScratchpadName(name.to_string()));
            }
            Ok(ScratchpadAction::Close {
                name: name.to_string(),
                target,
            })
        }
        _ => Err(ParseError::InvalidArgs(format!(
            "Unknown scratchpad action: {}",
            action
        ))),
    }
}

/// Extract a string value from a KDL child node (e.g. `width "60"` → "60", `y "10%"` → "10%").
fn parse_string_child(children: Option<&kdl::KdlDocument>, key: &str) -> Option<String> {
    children?
        .get(key)?
        .entries()
        .first()?
        .value()
        .as_string()
        .map(|s| s.to_string())
}

/// Parse a vertical axis keyword.
fn parse_vertical(s: &str) -> Option<AxisOrigin> {
    match s {
        "top" => Some(AxisOrigin::Start),
        "center" => Some(AxisOrigin::Center),
        "bottom" => Some(AxisOrigin::End),
        _ => None,
    }
}

/// Parse a horizontal axis keyword.
fn parse_horizontal(s: &str) -> Option<AxisOrigin> {
    match s {
        "left" => Some(AxisOrigin::Start),
        "center" => Some(AxisOrigin::Center),
        "right" => Some(AxisOrigin::End),
        _ => None,
    }
}

/// Parse an `origin` child node.
///
/// Accepts one arg (applied to both axes) or two args (vertical, horizontal).
fn parse_origin(children: Option<&kdl::KdlDocument>) -> Result<Origin, Option<String>> {
    let node = match children.and_then(|c| c.get("origin")) {
        Some(n) => n,
        None => return Ok(Origin::default()),
    };

    let args: Vec<&str> = node
        .entries()
        .iter()
        .filter_map(|e| e.value().as_string())
        .collect();

    match args.len() {
        1 => {
            // Single arg: try as vertical keyword first ("top"/"center"/"bottom"),
            // fall back to horizontal-only keywords ("left"/"right").
            if let Some(v) = parse_vertical(args[0]) {
                Ok(Origin {
                    vertical: v,
                    horizontal: if v == AxisOrigin::Center {
                        AxisOrigin::Center
                    } else {
                        AxisOrigin::Start
                    },
                })
            } else if let Some(h) = parse_horizontal(args[0]) {
                Ok(Origin {
                    vertical: AxisOrigin::Start,
                    horizontal: h,
                })
            } else {
                Err(Some(format!("Invalid origin value: '{}'", args[0])))
            }
        }
        2 => {
            let v = parse_vertical(args[0])
                .ok_or_else(|| Some(format!("Invalid vertical origin: '{}'", args[0])))?;
            let h = parse_horizontal(args[1])
                .ok_or_else(|| Some(format!("Invalid horizontal origin: '{}'", args[1])))?;
            Ok(Origin {
                vertical: v,
                horizontal: h,
            })
        }
        _ => Err(Some("origin expects 1 or 2 arguments".to_string())),
    }
}

fn canonical_input_mode_name(mode: &str) -> Result<String, String> {
    InputMode::from_str(mode)
        .map_err(|_| format!("Invalid keybind mode: '{}'", mode))?;
    let mode = mode.to_ascii_lowercase();
    if ALL_KEYBIND_MODES.contains(&mode.as_str()) {
        Ok(mode)
    } else {
        Err(format!("Unsupported keybind mode: '{}'", mode))
    }
}

fn validate_key(key: &str) -> Result<(), String> {
    KeyWithModifier::from_str(key)
        .map(|_| ())
        .map_err(|_| format!("Invalid keybind key: '{}'", key))
}

fn parse_input_mode(mode: &str) -> Result<InputMode, String> {
    canonical_input_mode_name(mode).and_then(|mode| {
        InputMode::from_str(&mode).map_err(|_| format!("Invalid keybind mode: '{}'", mode))
    })
}

fn parse_key_with_modifier(key: &str) -> Result<KeyWithModifier, String> {
    KeyWithModifier::from_str(key).map_err(|_| format!("Invalid keybind key: '{}'", key))
}

fn node_string_args(node: &kdl::KdlNode) -> Vec<String> {
    node.entries()
        .iter()
        .filter_map(|entry| entry.value().as_string())
        .map(ToString::to_string)
        .collect()
}

fn expanded_keybind_modes(block: &kdl::KdlNode) -> Result<Vec<String>, String> {
    let block_name = block.name().value();
    match block_name {
        "shared" => Ok(ALL_KEYBIND_MODES.iter().map(|mode| (*mode).to_string()).collect()),
        "shared_except" => {
            let excluded = node_string_args(block)
                .iter()
                .map(|mode| canonical_input_mode_name(mode))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ALL_KEYBIND_MODES
                .iter()
                .filter(|mode| !excluded.iter().any(|excluded| excluded == **mode))
                .map(|mode| (*mode).to_string())
                .collect())
        }
        "shared_among" => {
            let modes = node_string_args(block)
                .iter()
                .map(|mode| canonical_input_mode_name(mode))
                .collect::<Result<Vec<_>, _>>()?;
            if modes.is_empty() {
                return Err("shared_among requires at least one mode".to_string());
            }
            Ok(modes)
        }
        mode => {
            Ok(vec![canonical_input_mode_name(mode)?])
        }
    }
}

fn parse_keybind_action(node: &kdl::KdlNode) -> Result<ScratchpadKeybindAction, String> {
    let action = node.name().value();
    match action {
        "Toggle" => Ok(ScratchpadKeybindAction::Toggle),
        "Show" => Ok(ScratchpadKeybindAction::Show),
        "Hide" => Ok(ScratchpadKeybindAction::Hide),
        "Close" => Ok(ScratchpadKeybindAction::Close),
        "SwitchToMode" => {
            let args = node_string_args(node);
            let mode = args
                .first()
                .ok_or_else(|| "SwitchToMode requires a mode".to_string())?;
            if args.len() != 1 {
                return Err("SwitchToMode requires exactly one mode".to_string());
            }
            Ok(ScratchpadKeybindAction::SwitchToMode(
                canonical_input_mode_name(mode)?,
            ))
        }
        _ => Err(format!("Unsupported scratchpad keybind action: '{}'", action)),
    }
}

fn parse_keybind_actions(bind_node: &kdl::KdlNode) -> Result<Vec<ScratchpadKeybindAction>, String> {
    let action_doc = bind_node
        .children()
        .ok_or_else(|| "bind requires an action block".to_string())?;
    let actions: Vec<ScratchpadKeybindAction> = action_doc
        .nodes()
        .iter()
        .map(parse_keybind_action)
        .collect::<Result<_, _>>()?;
    if actions.is_empty() {
        return Err("bind requires at least one action".to_string());
    }
    Ok(actions)
}

fn parse_scratchpad_keybinds(
    children: Option<&kdl::KdlDocument>,
) -> Result<Vec<ScratchpadKeybind>, String> {
    let Some(keybinds_node) = children.and_then(|c| c.get("keybinds")) else {
        return Ok(Vec::new());
    };
    let Some(keybinds_doc) = keybinds_node.children() else {
        return Ok(Vec::new());
    };

    let mut keybinds = Vec::new();
    for mode_block in keybinds_doc.nodes() {
        let modes = expanded_keybind_modes(mode_block)?;
        let mode_doc = mode_block
            .children()
            .ok_or_else(|| format!("keybind mode '{}' requires a block", mode_block.name().value()))?;

        for bind_node in mode_doc.nodes() {
            if bind_node.name().value() != "bind" {
                return Err(format!(
                    "Unknown keybind instruction: '{}'",
                    bind_node.name().value()
                ));
            }

            let keys = node_string_args(bind_node);
            if keys.is_empty() {
                return Err("bind requires at least one key".to_string());
            }
            for key in &keys {
                validate_key(key)?;
            }

            keybinds.push(ScratchpadKeybind {
                modes: modes.clone(),
                keys,
                actions: parse_keybind_actions(bind_node)?,
            });
        }
    }

    Ok(keybinds)
}

fn scratchpad_action_payload(name: &str, action: &str) -> String {
    format!("zellij-tools::scratchpad::{}::{}", action, name)
}

fn scratchpad_keybind_pipe_action(scratchpad_name: &str, action: &str, own_plugin_id: u32) -> Action {
    Action::KeybindPipe {
        name: Some("zellij-tools".to_string()),
        payload: Some(scratchpad_action_payload(scratchpad_name, action)),
        args: None,
        plugin: None,
        plugin_id: Some(own_plugin_id),
        configuration: None,
        launch_new: false,
        skip_cache: false,
        floating: None,
        in_place: None,
        cwd: None,
        pane_title: None,
    }
}

fn generate_keybind_actions(
    scratchpad_name: &str,
    action: &ScratchpadKeybindAction,
    own_plugin_id: u32,
) -> Result<Action, String> {
    match action {
        ScratchpadKeybindAction::Toggle => Ok(scratchpad_keybind_pipe_action(
            scratchpad_name,
            "toggle",
            own_plugin_id,
        )),
        ScratchpadKeybindAction::Show => Ok(scratchpad_keybind_pipe_action(
            scratchpad_name,
            "show",
            own_plugin_id,
        )),
        ScratchpadKeybindAction::Hide => Ok(scratchpad_keybind_pipe_action(
            scratchpad_name,
            "hide",
            own_plugin_id,
        )),
        ScratchpadKeybindAction::Close => Ok(scratchpad_keybind_pipe_action(
            scratchpad_name,
            "close",
            own_plugin_id,
        )),
        ScratchpadKeybindAction::SwitchToMode(mode) => Ok(Action::SwitchToMode {
            input_mode: parse_input_mode(mode)?,
        }),
    }
}

pub fn build_scratchpad_keybind_update(
    scratchpad_name: &str,
    keybinds: &[ScratchpadKeybind],
    own_plugin_id: u32,
    installed: &[ScratchpadKeybindUnbind],
) -> Result<(Vec<ScratchpadKeybindUnbind>, Vec<ScratchpadKeybindRebind>), String> {
    let mut rebinds = Vec::new();
    for keybind in keybinds {
        let actions = keybind
            .actions
            .iter()
            .map(|action| generate_keybind_actions(scratchpad_name, action, own_plugin_id))
            .collect::<Result<Vec<_>, _>>()?;

        for mode in &keybind.modes {
            let input_mode = parse_input_mode(mode)?;
            for key in &keybind.keys {
                rebinds.push((input_mode, parse_key_with_modifier(key)?, actions.clone()));
            }
        }
    }

    Ok((installed.to_vec(), rebinds))
}

pub fn build_scratchpad_keybind_reconfigure(
    configs: &HashMap<String, ScratchpadConfig>,
    own_plugin_id: u32,
    installed: &[ScratchpadKeybindUnbind],
) -> Result<(Vec<ScratchpadKeybindUnbind>, Vec<ScratchpadKeybindUnbind>, String), String> {
    let mut mode_binds: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut new_installed = Vec::new();
    let mut configs: Vec<(&String, &ScratchpadConfig)> = configs.iter().collect();
    configs.sort_by(|(left, _), (right, _)| left.cmp(right));

    for (scratchpad_name, config) in configs {
        for keybind in &config.keybinds {
            let actions = keybind
                .actions
                .iter()
                .map(|action| scratchpad_keybind_kdl_action(scratchpad_name, action, own_plugin_id))
                .collect::<Result<Vec<_>, _>>()?
                .join(" ");

            for mode in &keybind.modes {
                let input_mode = parse_input_mode(mode)?;
                let mode = canonical_input_mode_name(mode)?;
                for key in &keybind.keys {
                    let parsed_key = parse_key_with_modifier(key)?;
                    new_installed.push((input_mode, parsed_key));
                    mode_binds.entry(mode.clone()).or_default().push(format!(
                        "      bind \"{}\" {{ {} }}",
                        escape_kdl_string(key),
                        actions
                    ));
                }
            }
        }
    }

    let mut kdl = String::from("keybinds {\n");
    for (mode, binds) in mode_binds {
        kdl.push_str(&format!("  {} {{\n", mode));
        for bind in binds {
            kdl.push_str(&bind);
            kdl.push('\n');
        }
        kdl.push_str("  }\n");
    }
    kdl.push_str("}\n");

    Ok((installed.to_vec(), new_installed, kdl))
}

fn scratchpad_keybind_kdl_action(
    scratchpad_name: &str,
    action: &ScratchpadKeybindAction,
    own_plugin_id: u32,
) -> Result<String, String> {
    match action {
        ScratchpadKeybindAction::Toggle => Ok(message_plugin_id_kdl_action(
            own_plugin_id,
            &scratchpad_action_payload(scratchpad_name, "toggle"),
        )),
        ScratchpadKeybindAction::Show => Ok(message_plugin_id_kdl_action(
            own_plugin_id,
            &scratchpad_action_payload(scratchpad_name, "show"),
        )),
        ScratchpadKeybindAction::Hide => Ok(message_plugin_id_kdl_action(
            own_plugin_id,
            &scratchpad_action_payload(scratchpad_name, "hide"),
        )),
        ScratchpadKeybindAction::Close => Ok(message_plugin_id_kdl_action(
            own_plugin_id,
            &scratchpad_action_payload(scratchpad_name, "close"),
        )),
        ScratchpadKeybindAction::SwitchToMode(mode) => Ok(format!(
            "SwitchToMode \"{}\";",
            escape_kdl_string(&canonical_input_mode_name(mode)?)
        )),
    }
}

fn message_plugin_id_kdl_action(own_plugin_id: u32, payload: &str) -> String {
    format!(
        "MessagePluginId {} {{ name \"zellij-tools\"; payload \"{}\"; }};",
        own_plugin_id,
        escape_kdl_string(payload)
    )
}

fn escape_kdl_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
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

        let children = node.children();

        // Look for command child node
        let command = children
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

        // Parse optional floating pane coordinates (fixed or percent)
        let x = parse_string_child(children, "x");
        let y = parse_string_child(children, "y");
        let width = parse_string_child(children, "width");
        let height = parse_string_child(children, "height");
        let origin = parse_origin(children)
            .map_err(|e| e.unwrap_or_else(|| format!("Invalid origin in scratchpad '{}'", name)))?;
        let title = parse_string_child(children, "title");
        let cwd = parse_string_child(children, "cwd");
        let keybinds = parse_scratchpad_keybinds(children)?;

        configs.insert(
            name,
            ScratchpadConfig {
                command,
                x,
                y,
                width,
                height,
                origin,
                title,
                cwd,
                keybinds,
            },
        );
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
    fn args<'a>(strs: &[&'a str]) -> Vec<&'a str> {
        strs.to_vec()
    }

    #[test]
    fn parse_toggle_no_name() {
        let action = parse_scratchpad_action(&args(&["toggle"])).unwrap();
        assert!(matches!(action, ScratchpadAction::Toggle { name: None, .. }));
    }

    #[test]
    fn parse_toggle_with_name() {
        let action = parse_scratchpad_action(&args(&["toggle", "mypad"])).unwrap();
        assert!(matches!(
            action,
            ScratchpadAction::Toggle { name: Some(n), .. } if n == "mypad"
        ));
    }

    #[test]
    fn parse_show() {
        let action = parse_scratchpad_action(&args(&["show", "mypad"])).unwrap();
        assert!(matches!(
            action,
            ScratchpadAction::Show { name, .. } if name == "mypad"
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
            ScratchpadAction::Hide { name, .. } if name == "mypad"
        ));
    }

    #[test]
    fn parse_close() {
        let action = parse_scratchpad_action(&args(&["close", "mypad"])).unwrap();
        assert!(matches!(
            action,
            ScratchpadAction::Close { name, .. } if name == "mypad"
        ));
    }

    #[test]
    fn parse_action_with_tab_id_target() {
        let action = parse_scratchpad_action(&args(&["toggle", "term", "tab-id", "7"])).unwrap();
        match action {
            ScratchpadAction::Toggle { name, target } => {
                assert_eq!(name.as_deref(), Some("term"));
                assert_eq!(target.tab_id, Some(7));
                assert_eq!(target.source_pane, None);
            }
            other => panic!("expected toggle action, got {other:?}"),
        }
    }

    #[test]
    fn parse_action_with_source_pane_target() {
        let action = parse_scratchpad_action(&args(&[
            "show",
            "term",
            "source-pane",
            "terminal_12",
        ]))
        .unwrap();
        match action {
            ScratchpadAction::Show { target, .. } => {
                assert_eq!(target.tab_id, None);
                assert_eq!(target.source_pane, Some(PaneId::Terminal(12)));
            }
            other => panic!("expected show action, got {other:?}"),
        }
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

    #[test]
    fn parse_kdl_width_and_height_fixed() {
        let input = r#"
            term {
                command "sh" "-c" "echo hello"
                width "60"
                height "10"
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].width.as_deref(), Some("60"));
        assert_eq!(configs["term"].height.as_deref(), Some("10"));
    }

    #[test]
    fn parse_kdl_width_and_height_percent() {
        let input = r#"
            term {
                command "nu"
                width "80%"
                height "50%"
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].width.as_deref(), Some("80%"));
        assert_eq!(configs["term"].height.as_deref(), Some("50%"));
    }

    #[test]
    fn parse_kdl_x_and_y() {
        let input = r#"
            term {
                command "nu"
                x "5"
                y "10%"
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].x.as_deref(), Some("5"));
        assert_eq!(configs["term"].y.as_deref(), Some("10%"));
    }

    #[test]
    fn parse_kdl_all_coordinates() {
        let input = r#"
            term {
                command "nu"
                x "1"
                y "10%"
                width "200"
                height "50%"
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].x.as_deref(), Some("1"));
        assert_eq!(configs["term"].y.as_deref(), Some("10%"));
        assert_eq!(configs["term"].width.as_deref(), Some("200"));
        assert_eq!(configs["term"].height.as_deref(), Some("50%"));
    }

    #[test]
    fn parse_kdl_width_only() {
        let input = r#"term { command "nu"; width "80"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].width.as_deref(), Some("80"));
        assert_eq!(configs["term"].height, None);
    }

    #[test]
    fn parse_kdl_no_dimensions() {
        let input = r#"term { command "nu"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].x, None);
        assert_eq!(configs["term"].y, None);
        assert_eq!(configs["term"].width, None);
        assert_eq!(configs["term"].height, None);
        assert_eq!(configs["term"].origin, Origin::default());
    }

    // origin parsing tests

    #[test]
    fn parse_kdl_origin_center() {
        let input = r#"term { command "nu"; origin "center"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(
            configs["term"].origin,
            Origin {
                vertical: AxisOrigin::Center,
                horizontal: AxisOrigin::Center,
            }
        );
    }

    #[test]
    fn parse_kdl_origin_top() {
        let input = r#"term { command "nu"; origin "top"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(
            configs["term"].origin,
            Origin {
                vertical: AxisOrigin::Start,
                horizontal: AxisOrigin::Start,
            }
        );
    }

    #[test]
    fn parse_kdl_origin_bottom() {
        let input = r#"term { command "nu"; origin "bottom"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(
            configs["term"].origin,
            Origin {
                vertical: AxisOrigin::End,
                horizontal: AxisOrigin::Start,
            }
        );
    }

    #[test]
    fn parse_kdl_origin_right() {
        let input = r#"term { command "nu"; origin "right"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(
            configs["term"].origin,
            Origin {
                vertical: AxisOrigin::Start,
                horizontal: AxisOrigin::End,
            }
        );
    }

    #[test]
    fn parse_kdl_origin_two_args() {
        let input = r#"term { command "nu"; origin "bottom" "center"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(
            configs["term"].origin,
            Origin {
                vertical: AxisOrigin::End,
                horizontal: AxisOrigin::Center,
            }
        );
    }

    #[test]
    fn parse_kdl_origin_top_right() {
        let input = r#"term { command "nu"; origin "top" "right"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(
            configs["term"].origin,
            Origin {
                vertical: AxisOrigin::Start,
                horizontal: AxisOrigin::End,
            }
        );
    }

    #[test]
    fn parse_kdl_origin_invalid() {
        let input = r#"term { command "nu"; origin "middle"; }"#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    #[test]
    fn parse_kdl_origin_invalid_vertical_in_two_args() {
        let input = r#"term { command "nu"; origin "left" "right"; }"#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    #[test]
    fn parse_kdl_origin_too_many_args() {
        let input = r#"term { command "nu"; origin "top" "left" "extra"; }"#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    #[test]
    fn parse_kdl_no_origin_defaults_to_center() {
        let input = r#"term { command "nu"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].origin, Origin::default());
        assert_eq!(configs["term"].origin.vertical, AxisOrigin::Center);
        assert_eq!(configs["term"].origin.horizontal, AxisOrigin::Center);
    }

    #[test]
    fn parse_keybind_single_mode() {
        let input = r#"
            term {
                command "nu"
                keybinds {
                    locked {
                        bind "Ctrl Shift D" { Toggle; }
                    }
                }
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        let keybind = &configs["term"].keybinds[0];
        assert_eq!(keybind.modes, vec!["locked"]);
        assert_eq!(keybind.keys, vec!["Ctrl Shift D"]);
        assert_eq!(keybind.actions, vec![ScratchpadKeybindAction::Toggle]);
    }

    #[test]
    fn parse_keybind_multiple_keys() {
        let input = r#"
            term {
                command "nu"
                keybinds {
                    normal {
                        bind "Alt t" "Alt Enter" { Toggle; }
                    }
                }
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].keybinds[0].keys, vec!["Alt t", "Alt Enter"]);
    }

    #[test]
    fn parse_keybind_multiple_actions() {
        let input = r#"
            term {
                command "nu"
                keybinds {
                    locked {
                        bind "Ctrl Shift D" { Toggle; SwitchToMode "locked"; }
                    }
                }
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(
            configs["term"].keybinds[0].actions,
            vec![
                ScratchpadKeybindAction::Toggle,
                ScratchpadKeybindAction::SwitchToMode("locked".to_string())
            ]
        );
    }

    #[test]
    fn parse_keybind_shared() {
        let input = r#"
            term {
                command "nu"
                keybinds {
                    shared {
                        bind "Alt s" { Show; }
                    }
                }
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].keybinds[0].modes, ALL_KEYBIND_MODES);
    }

    #[test]
    fn parse_keybind_shared_except() {
        let input = r#"
            term {
                command "nu"
                keybinds {
                    shared_except "resize" "scroll" {
                        bind "Alt h" { Hide; }
                    }
                }
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        let modes = &configs["term"].keybinds[0].modes;
        assert!(!modes.contains(&"resize".to_string()));
        assert!(!modes.contains(&"scroll".to_string()));
        assert!(modes.contains(&"normal".to_string()));
        assert!(modes.contains(&"locked".to_string()));
    }

    #[test]
    fn parse_keybind_shared_among() {
        let input = r#"
            term {
                command "nu"
                keybinds {
                    shared_among "locked" "normal" {
                        bind "Alt s" { Show; }
                    }
                }
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].keybinds[0].modes, vec!["locked", "normal"]);
    }

    #[test]
    fn parse_keybind_rejects_unsupported_action() {
        let input = r#"
            term {
                command "nu"
                keybinds { locked { bind "Ctrl d" { NewPane; } } }
            }
        "#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    #[test]
    fn parse_keybind_rejects_missing_bind_key() {
        let input = r#"
            term {
                command "nu"
                keybinds { locked { bind { Toggle; } } }
            }
        "#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    #[test]
    fn parse_keybind_rejects_missing_action_block() {
        let input = r#"
            term {
                command "nu"
                keybinds { locked { bind "Ctrl d"; } }
            }
        "#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    #[test]
    fn parse_keybind_rejects_invalid_mode() {
        let input = r#"
            term {
                command "nu"
                keybinds { imaginary { bind "Ctrl d" { Toggle; } } }
            }
        "#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    #[test]
    fn parse_keybind_rejects_invalid_key() {
        let input = r#"
            term {
                command "nu"
                keybinds { locked { bind "Ctrl NotAKey" { Toggle; } } }
            }
        "#;
        assert!(parse_scratchpads_kdl(input).is_err());
    }

    fn keybind_with_actions(actions: Vec<ScratchpadKeybindAction>) -> Vec<ScratchpadKeybind> {
        vec![ScratchpadKeybind {
            modes: vec!["locked".to_string()],
            keys: vec!["Ctrl d".to_string()],
            actions,
        }]
    }

    #[test]
    fn generated_keybind_uses_keybind_pipe_action() {
        let keybinds = keybind_with_actions(vec![ScratchpadKeybindAction::Toggle]);
        let (_, rebinds) = build_scratchpad_keybind_update("term", &keybinds, 7, &[]).unwrap();

        assert!(matches!(rebinds[0].2[0], Action::KeybindPipe { .. }));
    }

    #[test]
    fn generated_keybind_targets_own_plugin_id() {
        let keybinds = keybind_with_actions(vec![ScratchpadKeybindAction::Toggle]);
        let (_, rebinds) = build_scratchpad_keybind_update("term", &keybinds, 42, &[]).unwrap();

        match &rebinds[0].2[0] {
            Action::KeybindPipe {
                plugin, plugin_id, ..
            } => {
                assert_eq!(plugin, &None);
                assert_eq!(plugin_id, &Some(42));
            }
            action => panic!("expected KeybindPipe, got {action:?}"),
        }
    }

    #[test]
    fn generated_keybind_payload_contains_scratchpad_name_and_action() {
        let keybinds = keybind_with_actions(vec![ScratchpadKeybindAction::Hide]);
        let (_, rebinds) = build_scratchpad_keybind_update("term", &keybinds, 42, &[]).unwrap();

        match &rebinds[0].2[0] {
            Action::KeybindPipe { payload, .. } => {
                assert_eq!(payload.as_deref(), Some("zellij-tools::scratchpad::hide::term"));
            }
            action => panic!("expected KeybindPipe, got {action:?}"),
        }
    }

    #[test]
    fn generated_keybind_switch_to_mode_becomes_zellij_action() {
        let keybinds = keybind_with_actions(vec![
            ScratchpadKeybindAction::Toggle,
            ScratchpadKeybindAction::SwitchToMode("locked".to_string()),
        ]);
        let (_, rebinds) = build_scratchpad_keybind_update("term", &keybinds, 42, &[]).unwrap();

        assert!(matches!(
            rebinds[0].2[1],
            Action::SwitchToMode {
                input_mode: InputMode::Locked
            }
        ));
    }

    #[test]
    fn generated_unbind_list_matches_installed_keys() {
        let installed = vec![
            (
                InputMode::Locked,
                KeyWithModifier::from_str("Ctrl d").unwrap(),
            ),
            (InputMode::Normal, KeyWithModifier::from_str("Alt t").unwrap()),
        ];

        let keybinds = keybind_with_actions(vec![ScratchpadKeybindAction::Toggle]);
        let (unbinds, _) = build_scratchpad_keybind_update("term", &keybinds, 42, &installed).unwrap();

        assert_eq!(unbinds, installed);
    }

    #[test]
    fn generated_reconfigure_uses_message_plugin_id_kdl() {
        let configs = parse_scratchpads_kdl(
            r#"
                term {
                    command "nu"
                    keybinds {
                        locked {
                            bind "Ctrl d" { Toggle; SwitchToMode "locked"; }
                        }
                    }
                }
            "#,
        )
        .unwrap();

        let (_, installed, kdl) = build_scratchpad_keybind_reconfigure(&configs, 42, &[]).unwrap();

        assert_eq!(installed.len(), 1);
        assert!(kdl.contains("MessagePluginId 42"));
        assert!(kdl.contains("payload \"zellij-tools::scratchpad::toggle::term\""));
        assert!(kdl.contains("SwitchToMode \"locked\""));
    }

    // resolve_coordinates tests

    #[test]
    fn resolve_start_origin_passes_through() {
        let config = ScratchpadConfig {
            command: vec!["nu".into()],
            x: Some("10".into()),
            y: Some("20%".into()),
            width: Some("80".into()),
            height: Some("50%".into()),
            origin: Origin {
                vertical: AxisOrigin::Start,
                horizontal: AxisOrigin::Start,
            },
            title: None,
            cwd: None,
            keybinds: Vec::new(),
        };
        let resolved = config.resolve_coordinates(200, 50);
        assert_eq!(resolved.x.as_deref(), Some("10"));
        assert_eq!(resolved.y.as_deref(), Some("20%"));
        assert_eq!(resolved.width.as_deref(), Some("80"));
        assert_eq!(resolved.height.as_deref(), Some("50%"));
    }

    #[test]
    fn resolve_center_fixed_size() {
        // 80-col pane on 200-col viewport → x = (200-80)/2 = 60
        // 24-row pane on 50-row viewport  → y = (50-24)/2  = 13
        let config = ScratchpadConfig {
            command: vec!["nu".into()],
            x: None,
            y: None,
            width: Some("80".into()),
            height: Some("24".into()),
            origin: Origin {
                vertical: AxisOrigin::Center,
                horizontal: AxisOrigin::Center,
            },
            title: None,
            cwd: None,
            keybinds: Vec::new(),
        };
        let resolved = config.resolve_coordinates(200, 50);
        assert_eq!(resolved.x.as_deref(), Some("60"));
        assert_eq!(resolved.y.as_deref(), Some("13"));
    }

    #[test]
    fn resolve_center_percent_size() {
        // width 50% of 200 = 100 → x = (200-100)/2 = 50
        // height 50% of 50 = 25  → y = (50-25)/2  = 12
        let config = ScratchpadConfig {
            command: vec!["nu".into()],
            x: None,
            y: None,
            width: Some("50%".into()),
            height: Some("50%".into()),
            origin: Origin {
                vertical: AxisOrigin::Center,
                horizontal: AxisOrigin::Center,
            },
            title: None,
            cwd: None,
            keybinds: Vec::new(),
        };
        let resolved = config.resolve_coordinates(200, 50);
        assert_eq!(resolved.x.as_deref(), Some("50"));
        assert_eq!(resolved.y.as_deref(), Some("12"));
    }

    #[test]
    fn resolve_bottom_right() {
        // width 80, viewport 200 → x = 200 - 80 - 0 = 120
        // height 10, viewport 50 → y = 50 - 10 - 0  = 40
        let config = ScratchpadConfig {
            command: vec!["nu".into()],
            x: None,
            y: None,
            width: Some("80".into()),
            height: Some("10".into()),
            origin: Origin {
                vertical: AxisOrigin::End,
                horizontal: AxisOrigin::End,
            },
            title: None,
            cwd: None,
            keybinds: Vec::new(),
        };
        let resolved = config.resolve_coordinates(200, 50);
        assert_eq!(resolved.x.as_deref(), Some("120"));
        assert_eq!(resolved.y.as_deref(), Some("40"));
    }

    #[test]
    fn resolve_bottom_center_full_width() {
        // origin "bottom" "center", width 100%, height 30, viewport 200x50
        // x = (200-200)/2 + 0 = 0
        // y = 50 - 30 - 0 = 20
        let config = ScratchpadConfig {
            command: vec!["nu".into()],
            x: None,
            y: None,
            width: Some("100%".into()),
            height: Some("30".into()),
            origin: Origin {
                vertical: AxisOrigin::End,
                horizontal: AxisOrigin::Center,
            },
            title: None,
            cwd: None,
            keybinds: Vec::new(),
        };
        let resolved = config.resolve_coordinates(200, 50);
        assert_eq!(resolved.x.as_deref(), Some("0"));
        assert_eq!(resolved.y.as_deref(), Some("20"));
        assert_eq!(resolved.width.as_deref(), Some("100%"));
        assert_eq!(resolved.height.as_deref(), Some("30"));
    }

    #[test]
    fn resolve_bottom_with_offset() {
        // origin "bottom", y "5", height "10", viewport rows 50
        // y = 50 - 10 - 5 = 35
        let config = ScratchpadConfig {
            command: vec!["nu".into()],
            x: None,
            y: Some("5".into()),
            width: None,
            height: Some("10".into()),
            origin: Origin {
                vertical: AxisOrigin::End,
                horizontal: AxisOrigin::Start,
            },
            title: None,
            cwd: None,
            keybinds: Vec::new(),
        };
        let resolved = config.resolve_coordinates(200, 50);
        assert_eq!(resolved.y.as_deref(), Some("35"));
    }

    #[test]
    fn resolve_center_with_offset() {
        // origin center, x "5", width "80", viewport 200
        // x = (200-80)/2 + 5 = 65
        let config = ScratchpadConfig {
            command: vec!["nu".into()],
            x: Some("5".into()),
            y: None,
            width: Some("80".into()),
            height: None,
            origin: Origin {
                vertical: AxisOrigin::Center,
                horizontal: AxisOrigin::Center,
            },
            title: None,
            cwd: None,
            keybinds: Vec::new(),
        };
        let resolved = config.resolve_coordinates(200, 50);
        assert_eq!(resolved.x.as_deref(), Some("65"));
    }

    // title and cwd parsing tests

    #[test]
    fn parse_kdl_title() {
        let input = r#"term { command "nu"; title "My Terminal"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].title.as_deref(), Some("My Terminal"));
    }

    #[test]
    fn parse_kdl_cwd() {
        let input = r#"term { command "nu"; cwd "/home/user/projects"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].cwd.as_deref(), Some("/home/user/projects"));
    }

    #[test]
    fn parse_kdl_title_and_cwd() {
        let input = r#"
            term {
                command "nu"
                title "Dev Shell"
                cwd "/tmp"
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].title.as_deref(), Some("Dev Shell"));
        assert_eq!(configs["term"].cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn parse_kdl_no_title_or_cwd() {
        let input = r#"term { command "nu"; }"#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        assert_eq!(configs["term"].title, None);
        assert_eq!(configs["term"].cwd, None);
    }

    #[test]
    fn parse_kdl_all_options() {
        let input = r#"
            term {
                command "nu"
                x "5"
                y "10"
                width "80"
                height "24"
                origin "top" "left"
                title "Full Config"
                cwd "/home/user"
            }
        "#;
        let configs = parse_scratchpads_kdl(input).unwrap();
        let c = &configs["term"];
        assert_eq!(c.command, vec!["nu"]);
        assert_eq!(c.x.as_deref(), Some("5"));
        assert_eq!(c.y.as_deref(), Some("10"));
        assert_eq!(c.width.as_deref(), Some("80"));
        assert_eq!(c.height.as_deref(), Some("24"));
        assert_eq!(c.title.as_deref(), Some("Full Config"));
        assert_eq!(c.cwd.as_deref(), Some("/home/user"));
    }
}
