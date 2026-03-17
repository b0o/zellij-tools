use std::collections::HashMap;

use serde::Serialize;
use zellij_tile::prelude::PaneInfo;

use super::ScratchpadManager;
use crate::stable_tabs::StableTabId;

/// Query parameters for scratchpad list.
pub struct ScratchpadListQuery {
    /// Only include scratchpads with these names (empty = all).
    pub names: Vec<String>,
    /// Only include instances on this tab (None = all tabs).
    pub tab_id: Option<StableTabId>,
    /// Include full pane info for each instance.
    pub full: bool,
}

/// A single scratchpad entry in the list output.
#[derive(Debug, Serialize)]
pub struct ScratchpadListEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    pub orphaned: bool,
    pub instances: Vec<ScratchpadInstanceInfo>,
}

/// Info about a scratchpad instance on a specific tab.
#[derive(Debug, Serialize)]
pub struct ScratchpadInstanceInfo {
    pub tab_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_position: Option<usize>,
    pub pane_id: u32,
    pub visible: bool,
    /// Full pane details (only present when `--full` is requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane: Option<PaneNodeCompact>,
}

/// Pane details included in full mode.
#[derive(Debug, Serialize)]
pub struct PaneNodeCompact {
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
    pub pane_y: usize,
    pub pane_rows: usize,
    pub pane_columns: usize,
    pub terminal_command: Option<String>,
    pub plugin_url: Option<String>,
    pub is_selectable: bool,
}

impl PaneNodeCompact {
    fn from_pane_info(p: &PaneInfo) -> Self {
        Self {
            is_plugin: p.is_plugin,
            is_focused: p.is_focused,
            is_fullscreen: p.is_fullscreen,
            is_floating: p.is_floating,
            is_suppressed: p.is_suppressed,
            title: p.title.clone(),
            exited: p.exited,
            exit_status: p.exit_status,
            is_held: p.is_held,
            pane_x: p.pane_x,
            pane_y: p.pane_y,
            pane_rows: p.pane_rows,
            pane_columns: p.pane_columns,
            terminal_command: p.terminal_command.clone(),
            plugin_url: p.plugin_url.clone(),
            is_selectable: p.is_selectable,
        }
    }
}

impl ScratchpadManager {
    /// Build a list of scratchpads matching the query.
    pub fn list(
        &self,
        query: &ScratchpadListQuery,
        pane_manifest: &HashMap<usize, Vec<PaneInfo>>,
        stable_tab_to_position: &HashMap<StableTabId, usize>,
    ) -> Vec<ScratchpadListEntry> {
        // Build a lookup: pane_id -> &PaneInfo (across all tabs)
        let pane_lookup: HashMap<u32, &PaneInfo> = pane_manifest
            .values()
            .flatten()
            .map(|p| (p.id, p))
            .collect();

        // Build reverse lookup: stable_tab_id -> tab_position
        let position_lookup = stable_tab_to_position;

        let name_filter: Option<std::collections::HashSet<&str>> = if query.names.is_empty() {
            None
        } else {
            Some(query.names.iter().map(|s| s.as_str()).collect())
        };

        let mut entries = Vec::new();

        // Configured scratchpads
        for (name, config) in &self.configs {
            if let Some(ref filter) = name_filter {
                if !filter.contains(name.as_str()) {
                    continue;
                }
            }

            let instances = self.build_instances(
                name,
                query.tab_id,
                query.full,
                &pane_lookup,
                position_lookup,
            );

            entries.push(ScratchpadListEntry {
                name: name.clone(),
                command: Some(config.command.clone()),
                orphaned: false,
                instances,
            });
        }

        // Orphaned scratchpads (removed from config but still have panes)
        for (name, orphaned_tabs) in &self.orphaned {
            // Skip if already included as a configured scratchpad
            if self.configs.contains_key(name) {
                continue;
            }
            if let Some(ref filter) = name_filter {
                if !filter.contains(name.as_str()) {
                    continue;
                }
            }

            let instances = self.build_orphaned_instances(
                name,
                orphaned_tabs,
                query.tab_id,
                query.full,
                &pane_lookup,
                position_lookup,
            );

            entries.push(ScratchpadListEntry {
                name: name.clone(),
                command: None,
                orphaned: true,
                instances,
            });
        }

        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    fn build_instances(
        &self,
        name: &str,
        tab_filter: Option<StableTabId>,
        full: bool,
        pane_lookup: &HashMap<u32, &PaneInfo>,
        position_lookup: &HashMap<StableTabId, usize>,
    ) -> Vec<ScratchpadInstanceInfo> {
        let Some(tab_panes) = self.panes.get(name) else {
            return Vec::new();
        };

        tab_panes
            .iter()
            .filter(|(&stable_id, _)| tab_filter.is_none() || tab_filter == Some(stable_id))
            .map(|(&stable_id, &pane_id)| {
                let pane_info = pane_lookup.get(&pane_id);
                let visible = pane_info
                    .map(|p| p.is_floating && !p.is_suppressed && !p.exited && !p.is_held)
                    .unwrap_or(false);

                ScratchpadInstanceInfo {
                    tab_id: stable_id,
                    tab_position: position_lookup.get(&stable_id).copied(),
                    pane_id,
                    visible,
                    pane: if full {
                        pane_info.map(|p| PaneNodeCompact::from_pane_info(p))
                    } else {
                        None
                    },
                }
            })
            .collect()
    }

    fn build_orphaned_instances(
        &self,
        name: &str,
        _orphaned_tabs: &std::collections::HashSet<StableTabId>,
        tab_filter: Option<StableTabId>,
        full: bool,
        pane_lookup: &HashMap<u32, &PaneInfo>,
        position_lookup: &HashMap<StableTabId, usize>,
    ) -> Vec<ScratchpadInstanceInfo> {
        // Orphaned scratchpads still have entries in self.panes
        self.build_instances(name, tab_filter, full, pane_lookup, position_lookup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratchpad::config::ScratchpadConfig;
    use crate::scratchpad::Origin;

    fn make_config(cmd: &str) -> ScratchpadConfig {
        ScratchpadConfig {
            command: vec![cmd.to_string()],
            x: None,
            y: None,
            width: None,
            height: None,
            origin: Origin::default(),
            title: None,
            cwd: None,
        }
    }

    fn make_pane_info(id: u32, is_floating: bool, is_suppressed: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_floating,
            is_suppressed,
            ..Default::default()
        }
    }

    #[test]
    fn list_empty_manager() {
        let manager = ScratchpadManager::new(HashMap::new());
        let manifest = HashMap::new();
        let positions = HashMap::new();
        let query = ScratchpadListQuery {
            names: vec![],
            tab_id: None,
            full: false,
        };
        let entries = manager.list(&query, &manifest, &positions);
        assert!(entries.is_empty());
    }

    #[test]
    fn list_configured_scratchpad_no_instances() {
        let configs = HashMap::from([("term".to_string(), make_config("nu"))]);
        let manager = ScratchpadManager::new(configs);
        let manifest = HashMap::new();
        let positions = HashMap::new();
        let query = ScratchpadListQuery {
            names: vec![],
            tab_id: None,
            full: false,
        };
        let entries = manager.list(&query, &manifest, &positions);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "term");
        assert_eq!(entries[0].command.as_deref(), Some(&["nu".to_string()][..]));
        assert!(!entries[0].orphaned);
        assert!(entries[0].instances.is_empty());
    }

    #[test]
    fn list_with_active_instance() {
        let configs = HashMap::from([("term".to_string(), make_config("nu"))]);
        let mut manager = ScratchpadManager::new(configs);

        // Simulate registration: directly insert pane mapping
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(1, 42);

        let mut manifest: HashMap<usize, Vec<PaneInfo>> = HashMap::new();
        manifest.insert(0, vec![make_pane_info(42, true, false)]);
        let positions = HashMap::from([(1_u64, 0_usize)]);

        let query = ScratchpadListQuery {
            names: vec![],
            tab_id: None,
            full: false,
        };
        let entries = manager.list(&query, &manifest, &positions);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].instances.len(), 1);
        assert_eq!(entries[0].instances[0].tab_id, 1);
        assert_eq!(entries[0].instances[0].pane_id, 42);
        assert!(entries[0].instances[0].visible);
        assert!(entries[0].instances[0].pane.is_none());
    }

    #[test]
    fn list_with_hidden_instance() {
        let configs = HashMap::from([("term".to_string(), make_config("nu"))]);
        let mut manager = ScratchpadManager::new(configs);
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(1, 42);

        let mut manifest: HashMap<usize, Vec<PaneInfo>> = HashMap::new();
        manifest.insert(0, vec![make_pane_info(42, true, true)]); // suppressed
        let positions = HashMap::from([(1_u64, 0_usize)]);

        let query = ScratchpadListQuery {
            names: vec![],
            tab_id: None,
            full: false,
        };
        let entries = manager.list(&query, &manifest, &positions);
        assert!(!entries[0].instances[0].visible);
    }

    #[test]
    fn list_with_name_filter() {
        let configs = HashMap::from([
            ("term".to_string(), make_config("nu")),
            ("htop".to_string(), make_config("htop")),
        ]);
        let manager = ScratchpadManager::new(configs);
        let manifest = HashMap::new();
        let positions = HashMap::new();
        let query = ScratchpadListQuery {
            names: vec!["term".to_string()],
            tab_id: None,
            full: false,
        };
        let entries = manager.list(&query, &manifest, &positions);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "term");
    }

    #[test]
    fn list_with_tab_filter() {
        let configs = HashMap::from([("term".to_string(), make_config("nu"))]);
        let mut manager = ScratchpadManager::new(configs);
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(1, 42);
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(2, 99);

        let mut manifest: HashMap<usize, Vec<PaneInfo>> = HashMap::new();
        manifest.insert(0, vec![make_pane_info(42, true, false)]);
        manifest.insert(1, vec![make_pane_info(99, true, false)]);
        let positions = HashMap::from([(1_u64, 0_usize), (2_u64, 1_usize)]);

        let query = ScratchpadListQuery {
            names: vec![],
            tab_id: Some(1),
            full: false,
        };
        let entries = manager.list(&query, &manifest, &positions);
        assert_eq!(entries[0].instances.len(), 1);
        assert_eq!(entries[0].instances[0].tab_id, 1);
    }

    #[test]
    fn list_full_includes_pane_info() {
        let configs = HashMap::from([("term".to_string(), make_config("nu"))]);
        let mut manager = ScratchpadManager::new(configs);
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(1, 42);

        let mut pane = make_pane_info(42, true, false);
        pane.title = "my terminal".to_string();
        let mut manifest: HashMap<usize, Vec<PaneInfo>> = HashMap::new();
        manifest.insert(0, vec![pane]);
        let positions = HashMap::from([(1_u64, 0_usize)]);

        let query = ScratchpadListQuery {
            names: vec![],
            tab_id: None,
            full: true,
        };
        let entries = manager.list(&query, &manifest, &positions);
        let pane_node = entries[0].instances[0].pane.as_ref().unwrap();
        assert_eq!(pane_node.title, "my terminal");
        assert!(pane_node.is_floating);
    }

    #[test]
    fn list_entries_sorted_by_name() {
        let configs = HashMap::from([
            ("zebra".to_string(), make_config("z")),
            ("alpha".to_string(), make_config("a")),
            ("mid".to_string(), make_config("m")),
        ]);
        let manager = ScratchpadManager::new(configs);
        let manifest = HashMap::new();
        let positions = HashMap::new();
        let query = ScratchpadListQuery {
            names: vec![],
            tab_id: None,
            full: false,
        };
        let entries = manager.list(&query, &manifest, &positions);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mid", "zebra"]);
    }
}
