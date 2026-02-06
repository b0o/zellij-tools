use std::collections::HashMap;

use serde::Serialize;

use crate::stable_tabs::StableTabTracker;

/// The full session tree returned by the `tree` command.
#[derive(Debug, Serialize)]
pub struct SessionTree {
    pub tabs: Vec<TabNode>,
}

/// A tab in the session tree, with all zellij TabInfo fields plus extras.
#[derive(Debug, Serialize)]
pub struct TabNode {
    // Our stable ID
    pub stable_id: Option<u64>,

    // All zellij TabInfo fields
    pub position: usize,
    pub name: String,
    pub active: bool,
    pub panes_to_hide: usize,
    pub is_fullscreen_active: bool,
    pub is_sync_panes_active: bool,
    pub are_floating_panes_visible: bool,
    pub active_swap_layout_name: Option<String>,
    pub is_swap_layout_dirty: bool,

    // Nested panes
    pub tiled_panes: Vec<PaneNode>,
    pub floating_panes: Vec<PaneNode>,
}

/// A pane in the session tree, with all zellij PaneInfo fields.
#[derive(Debug, Serialize)]
pub struct PaneNode {
    pub id: u32,
    pub is_plugin: bool,
    pub is_focused: bool,
    pub is_fullscreen: bool,
    pub is_floating: bool,
    pub is_suppressed: bool,
    pub title: String,
    pub exited: bool,
    pub exit_status: Option<i32>,
    pub is_held: bool,
    pub pane_x: usize,
    pub pane_content_x: usize,
    pub pane_y: usize,
    pub pane_content_y: usize,
    pub pane_rows: usize,
    pub pane_content_rows: usize,
    pub pane_columns: usize,
    pub pane_content_columns: usize,
    pub cursor_coordinates_in_pane: Option<(usize, usize)>,
    pub terminal_command: Option<String>,
    pub plugin_url: Option<String>,
    pub is_selectable: bool,
}

/// Build a session tree from the current plugin state.
pub fn build_tree(
    tab_infos: &[zellij_tile::prelude::TabInfo],
    pane_manifest: &HashMap<usize, Vec<zellij_tile::prelude::PaneInfo>>,
    tab_tracker: &StableTabTracker,
) -> SessionTree {
    let mut tabs: Vec<TabNode> = tab_infos
        .iter()
        .map(|tab| {
            let stable_id = tab_tracker.get_stable_id(tab.position);
            let panes = pane_manifest.get(&tab.position);

            let mut tiled_panes = Vec::new();
            let mut floating_panes = Vec::new();

            if let Some(panes) = panes {
                for pane in panes {
                    let node = PaneNode {
                        id: pane.id,
                        is_plugin: pane.is_plugin,
                        is_focused: pane.is_focused,
                        is_fullscreen: pane.is_fullscreen,
                        is_floating: pane.is_floating,
                        is_suppressed: pane.is_suppressed,
                        title: pane.title.clone(),
                        exited: pane.exited,
                        exit_status: pane.exit_status,
                        is_held: pane.is_held,
                        pane_x: pane.pane_x,
                        pane_content_x: pane.pane_content_x,
                        pane_y: pane.pane_y,
                        pane_content_y: pane.pane_content_y,
                        pane_rows: pane.pane_rows,
                        pane_content_rows: pane.pane_content_rows,
                        pane_columns: pane.pane_columns,
                        pane_content_columns: pane.pane_content_columns,
                        cursor_coordinates_in_pane: pane.cursor_coordinates_in_pane,
                        terminal_command: pane.terminal_command.clone(),
                        plugin_url: pane.plugin_url.clone(),
                        is_selectable: pane.is_selectable,
                    };
                    if pane.is_floating {
                        floating_panes.push(node);
                    } else {
                        tiled_panes.push(node);
                    }
                }
            }

            TabNode {
                stable_id,
                position: tab.position,
                name: tab.name.clone(),
                active: tab.active,
                panes_to_hide: tab.panes_to_hide,
                is_fullscreen_active: tab.is_fullscreen_active,
                is_sync_panes_active: tab.is_sync_panes_active,
                are_floating_panes_visible: tab.are_floating_panes_visible,
                active_swap_layout_name: tab.active_swap_layout_name.clone(),
                is_swap_layout_dirty: tab.is_swap_layout_dirty,
                tiled_panes,
                floating_panes,
            }
        })
        .collect();

    // Sort by position for deterministic output
    tabs.sort_by_key(|t| t.position);

    SessionTree { tabs }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_tree_serializes_to_json() {
        let tree = SessionTree {
            tabs: vec![TabNode {
                stable_id: Some(1),
                position: 0,
                name: "tab1".to_string(),
                active: true,
                panes_to_hide: 0,
                is_fullscreen_active: false,
                is_sync_panes_active: false,
                are_floating_panes_visible: false,
                active_swap_layout_name: None,
                is_swap_layout_dirty: false,
                tiled_panes: vec![PaneNode {
                    id: 0,
                    is_plugin: false,
                    is_focused: true,
                    is_fullscreen: false,
                    is_floating: false,
                    is_suppressed: false,
                    title: "zsh".to_string(),
                    exited: false,
                    exit_status: None,
                    is_held: false,
                    pane_x: 0,
                    pane_content_x: 1,
                    pane_y: 0,
                    pane_content_y: 1,
                    pane_rows: 50,
                    pane_content_rows: 48,
                    pane_columns: 200,
                    pane_content_columns: 198,
                    cursor_coordinates_in_pane: Some((10, 20)),
                    terminal_command: Some("zsh".to_string()),
                    plugin_url: None,
                    is_selectable: true,
                }],
                floating_panes: vec![],
            }],
        };

        let json = serde_json::to_string(&tree).unwrap();
        // Verify it parses back as valid JSON
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value["tabs"].is_array());
        assert_eq!(value["tabs"][0]["stable_id"], 1);
        assert_eq!(value["tabs"][0]["name"], "tab1");
        assert_eq!(value["tabs"][0]["tiled_panes"][0]["id"], 0);
        assert_eq!(value["tabs"][0]["tiled_panes"][0]["title"], "zsh");
        assert!(value["tabs"][0]["floating_panes"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn empty_tree_serializes() {
        let tree = SessionTree { tabs: vec![] };
        let json = serde_json::to_string(&tree).unwrap();
        assert_eq!(json, r#"{"tabs":[]}"#);
    }
}
