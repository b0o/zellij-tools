use std::collections::{BTreeMap, HashMap, HashSet};

use super::{ScratchpadContext, ScratchpadManager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScratchpadDisplayState {
    Visible,
    Hidden,
    Closed,
}

impl ScratchpadDisplayState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchpadStatusItem {
    pub name: String,
    pub title: String,
    pub state: ScratchpadDisplayState,
    pub pane_id: Option<u32>,
    pub tab_id: Option<usize>,
    pub tab_position: Option<usize>,
    pub is_focused: bool,
    pub is_mru: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScratchpadStatusSnapshot {
    pub current_items: Vec<ScratchpadStatusItem>,
    pub global_count_items: Vec<ScratchpadStatusItem>,
    pub global_items: Vec<ScratchpadStatusItem>,
}

struct LiveScratchpadInstance {
    pane_id: u32,
    tab_id: usize,
    tab_position: Option<usize>,
    state: ScratchpadDisplayState,
    is_focused: bool,
}

#[derive(Debug, Clone, Copy)]
struct PaneStatus {
    is_floating: bool,
    is_suppressed: bool,
    is_focused: bool,
    exited: bool,
    is_held: bool,
}

impl ScratchpadManager {
    pub fn status_snapshot(&self, ctx: &ScratchpadContext<'_>) -> ScratchpadStatusSnapshot {
        let pane_lookup = build_pane_lookup(ctx);
        self.status_snapshot_with_lookup(ctx, &pane_lookup)
    }

    /// Build current/global status and alphabetical items for every authoritative tab ID.
    /// Includes passive observations in a display-only view; legacy status and actions
    /// remain local-only. Local ownership takes precedence, including in-flight panes.
    /// Closed tab-local items retain their target tab identity. The caller must wait
    /// for the first authoritative tab snapshot; an empty tab map stays empty.
    /// Call `prune_passive_observations` before registry I/O and this projection.
    pub fn publication_snapshot(
        &self,
        ctx: &ScratchpadContext<'_>,
    ) -> (
        ScratchpadStatusSnapshot,
        BTreeMap<usize, Vec<ScratchpadStatusItem>>,
    ) {
        let pane_lookup = build_pane_lookup(ctx);
        let view = self.publication_view();
        let snapshot = view.status_snapshot_with_lookup(ctx, &pane_lookup);
        let tab_items = ctx
            .tab_id_to_position
            .iter()
            .map(|(&tab_id, &tab_position)| {
                let tab_ctx = ScratchpadContext {
                    current_tab_id: Some(tab_id),
                    current_tab_position: tab_position,
                    ..*ctx
                };
                let mru = view
                    .get_focused_scratchpad(&tab_ctx)
                    .or_else(|| view.get_last_focused_on_current_tab(&tab_ctx));
                let items = snapshot
                    .current_items
                    .iter()
                    .map(|configured| {
                        let mut item =
                            view.current_status_item(&configured.name, &tab_ctx, &pane_lookup);
                        item.tab_id = Some(tab_id);
                        item.tab_position = Some(tab_position);
                        item.is_mru = mru.as_deref() == Some(item.name.as_str());
                        item
                    })
                    .collect();
                (tab_id, items)
            })
            .collect();
        (snapshot, tab_items)
    }

    pub(super) fn publication_view(&self) -> Self {
        let mut view = Self::new(self.configs.clone());
        view.panes = self.panes.clone();
        view.focus_times = self.focus_times.clone();
        view.focus_counter = self.focus_counter;
        view.just_shown = self.just_shown;
        let mut pane_ids: HashSet<u32> = self
            .panes
            .values()
            .flat_map(|panes| panes.values().copied())
            .collect();
        for (name, panes) in &self.passive_panes {
            if !self.configs.contains_key(name) {
                continue;
            }
            for (&tab_id, &pane_id) in panes {
                if view
                    .panes
                    .get(name)
                    .is_some_and(|panes| panes.contains_key(&tab_id))
                    || !pane_ids.insert(pane_id)
                {
                    continue;
                }
                view.panes
                    .entry(name.clone())
                    .or_default()
                    .insert(tab_id, pane_id);
            }
        }
        for (&tab_id, (name, pane_id, local_counter)) in &self.passive_mru {
            if self.configs.contains_key(name)
                && view.panes.get(name).and_then(|panes| panes.get(&tab_id)) == Some(pane_id)
                && !self
                    .focus_times
                    .values()
                    .any(|times| times.get(&tab_id).is_some_and(|time| time > local_counter))
            {
                // Same-pane adoption does not constitute a new focus event.
                view.focus_times
                    .entry(name.clone())
                    .or_default()
                    .insert(tab_id, self.focus_counter.saturating_add(1));
            }
        }
        view
    }

    fn status_snapshot_with_lookup(
        &self,
        ctx: &ScratchpadContext<'_>,
        pane_lookup: &HashMap<(usize, u32), PaneStatus>,
    ) -> ScratchpadStatusSnapshot {
        let mut names: Vec<&String> = self.configs.keys().collect();
        names.sort();

        let mru = self
            .get_focused_scratchpad(ctx)
            .or_else(|| self.get_last_focused_on_current_tab(ctx));
        let current_items = names
            .iter()
            .map(|name| {
                let mut item = self.current_status_item(name, ctx, pane_lookup);
                item.is_mru = mru.as_deref() == Some(name.as_str());
                item
            })
            .collect();
        let global_count_items = names
            .iter()
            .flat_map(|name| self.global_count_status_items(name, ctx, pane_lookup))
            .collect();
        let global_items = names
            .iter()
            .map(|name| self.global_status_item(name, ctx, pane_lookup))
            .collect();

        ScratchpadStatusSnapshot {
            current_items,
            global_count_items,
            global_items,
        }
    }

    fn current_status_item(
        &self,
        name: &str,
        ctx: &ScratchpadContext<'_>,
        pane_lookup: &HashMap<(usize, u32), PaneStatus>,
    ) -> ScratchpadStatusItem {
        let title = self.status_title(name);
        let Some(tab_id) = ctx.current_tab_id else {
            return ScratchpadStatusItem::closed(name, title);
        };

        let instance = self
            .panes
            .get(name)
            .and_then(|panes| panes.get(&tab_id).copied())
            .and_then(|pane_id| self.live_instance(pane_id, tab_id, ctx, pane_lookup));

        match instance {
            Some(instance) => ScratchpadStatusItem::from_instance(name, title, instance),
            None => ScratchpadStatusItem::closed(name, title),
        }
    }

    fn global_status_item(
        &self,
        name: &str,
        ctx: &ScratchpadContext<'_>,
        pane_lookup: &HashMap<(usize, u32), PaneStatus>,
    ) -> ScratchpadStatusItem {
        let title = self.status_title(name);
        let mut instances = self
            .panes
            .get(name)
            .into_iter()
            .flat_map(|panes| panes.iter())
            .filter_map(|(&tab_id, &pane_id)| self.live_instance(pane_id, tab_id, ctx, pane_lookup))
            .collect::<Vec<_>>();

        if instances.is_empty() {
            return ScratchpadStatusItem::closed(name, title);
        }

        instances.sort_by(|left, right| {
            instance_rank(left)
                .cmp(&instance_rank(right))
                .then_with(|| left.pane_id.cmp(&right.pane_id))
        });
        let chosen = instances.remove(0);
        let state = if chosen.state == ScratchpadDisplayState::Visible {
            ScratchpadDisplayState::Visible
        } else {
            ScratchpadDisplayState::Hidden
        };

        ScratchpadStatusItem {
            name: name.to_string(),
            title,
            state,
            pane_id: Some(chosen.pane_id),
            tab_id: Some(chosen.tab_id),
            tab_position: chosen.tab_position,
            is_focused: chosen.is_focused,
            is_mru: false,
        }
    }

    fn global_count_status_items(
        &self,
        name: &str,
        ctx: &ScratchpadContext<'_>,
        pane_lookup: &HashMap<(usize, u32), PaneStatus>,
    ) -> Vec<ScratchpadStatusItem> {
        let title = self.status_title(name);
        let items = self
            .panes
            .get(name)
            .into_iter()
            .flat_map(|panes| panes.iter())
            .filter_map(|(&tab_id, &pane_id)| self.live_instance(pane_id, tab_id, ctx, pane_lookup))
            .map(|instance| ScratchpadStatusItem::from_instance(name, title.clone(), instance))
            .collect::<Vec<_>>();

        if items.is_empty() {
            vec![ScratchpadStatusItem::closed(name, title)]
        } else {
            items
        }
    }

    fn live_instance(
        &self,
        pane_id: u32,
        tab_id: usize,
        ctx: &ScratchpadContext<'_>,
        pane_lookup: &HashMap<(usize, u32), PaneStatus>,
    ) -> Option<LiveScratchpadInstance> {
        let tab_position = ctx.tab_id_to_position.get(&tab_id).copied();
        let pane =
            tab_position.and_then(|position| pane_lookup.get(&(position, pane_id)).copied())?;

        if pane.exited || pane.is_held {
            return None;
        }

        let is_focused = self.just_shown == Some(pane_id) || pane.is_focused;
        let state = if pane.is_floating && !pane.is_suppressed {
            ScratchpadDisplayState::Visible
        } else {
            ScratchpadDisplayState::Hidden
        };

        Some(LiveScratchpadInstance {
            pane_id,
            tab_id,
            tab_position,
            state,
            is_focused,
        })
    }

    fn status_title(&self, name: &str) -> String {
        self.configs
            .get(name)
            .and_then(|config| config.title.clone())
            .unwrap_or_else(|| name.to_string())
    }
}

impl ScratchpadStatusItem {
    fn closed(name: &str, title: String) -> Self {
        Self {
            name: name.to_string(),
            title,
            state: ScratchpadDisplayState::Closed,
            pane_id: None,
            tab_id: None,
            tab_position: None,
            is_focused: false,
            is_mru: false,
        }
    }

    fn from_instance(name: &str, title: String, instance: LiveScratchpadInstance) -> Self {
        Self {
            name: name.to_string(),
            title,
            state: instance.state,
            pane_id: Some(instance.pane_id),
            tab_id: Some(instance.tab_id),
            tab_position: instance.tab_position,
            is_focused: instance.is_focused,
            is_mru: false,
        }
    }
}

fn build_pane_lookup(ctx: &ScratchpadContext<'_>) -> HashMap<(usize, u32), PaneStatus> {
    ctx.pane_manifest
        .iter()
        .flat_map(|(&tab_position, panes)| {
            panes.iter().filter_map(move |pane| {
                (!pane.is_plugin).then_some((
                    (tab_position, pane.id),
                    PaneStatus {
                        is_floating: pane.is_floating,
                        is_suppressed: pane.is_suppressed,
                        is_focused: pane.is_focused,
                        exited: pane.exited,
                        is_held: pane.is_held,
                    },
                ))
            })
        })
        .collect()
}

fn instance_rank(instance: &LiveScratchpadInstance) -> (u8, usize) {
    let state_rank = match (instance.is_focused, instance.state) {
        (true, ScratchpadDisplayState::Visible) => 0,
        (false, ScratchpadDisplayState::Visible) => 1,
        (true, ScratchpadDisplayState::Hidden) => 2,
        (false, ScratchpadDisplayState::Hidden) => 3,
        (_, ScratchpadDisplayState::Closed) => 4,
    };
    (state_rank, instance.tab_position.unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use zellij_tile::prelude::PaneInfo;

    use super::*;
    use crate::scratchpad::{Origin, ScratchpadConfig};

    fn make_config(title: Option<&str>) -> ScratchpadConfig {
        ScratchpadConfig {
            command: vec!["bash".to_string()],
            x: None,
            y: None,
            width: None,
            height: None,
            origin: Origin::default(),
            title: title.map(ToString::to_string),
            cwd: None,
            keybinds: Vec::new(),
        }
    }

    fn make_context<'a>(
        pane_manifest: &'a HashMap<usize, Vec<PaneInfo>>,
        tab_id_to_position: &'a HashMap<usize, usize>,
    ) -> ScratchpadContext<'a> {
        ScratchpadContext {
            pane_manifest,
            current_tab_position: 0,
            current_tab_id: Some(10),
            are_floating_panes_visible: true,
            tab_id_to_position,
            viewport_cols: 100,
            viewport_rows: 40,
        }
    }

    fn make_pane(id: u32, suppressed: bool, focused: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_floating: true,
            is_suppressed: suppressed,
            is_focused: focused,
            ..Default::default()
        }
    }

    #[test]
    fn publication_keeps_tab_local_state_and_global_semantics_separate() {
        let mut manager = ScratchpadManager::new(HashMap::from([
            ("term".to_string(), make_config(None)),
            ("notes".to_string(), make_config(Some("Notes"))),
        ]));
        manager.panes = HashMap::from([
            ("term".to_string(), HashMap::from([(10, 42), (11, 43)])),
            ("notes".to_string(), HashMap::from([(11, 44)])),
        ]);
        manager.focus_times = HashMap::from([
            ("term".to_string(), HashMap::from([(10, 1), (11, 2)])),
            ("notes".to_string(), HashMap::from([(11, 3)])),
        ]);
        let manifest = HashMap::from([
            (0, vec![make_pane(42, false, true)]),
            (
                1,
                vec![make_pane(43, true, false), make_pane(44, true, false)],
            ),
        ]);
        let positions = HashMap::from([(10, 0), (11, 1), (12, 2)]);
        let ctx = make_context(&manifest, &positions);
        let (snapshot, tabs) = manager.publication_snapshot(&ctx);

        assert_eq!(snapshot, manager.status_snapshot(&ctx));
        assert_eq!(tabs.keys().copied().collect::<Vec<_>>(), vec![10, 11, 12]);
        assert_eq!(tabs[&10][0].name, "notes");
        assert_eq!(tabs[&10][1].state, ScratchpadDisplayState::Visible);
        assert_eq!(tabs[&10][1].pane_id, Some(42));
        assert!(tabs[&10][1].is_mru);
        assert_eq!(tabs[&11][1].state, ScratchpadDisplayState::Hidden);
        assert_eq!(tabs[&11][1].pane_id, Some(43));
        assert!(tabs[&11][0].is_mru);
        assert!(!tabs[&11][1].is_mru);
        assert_eq!(snapshot.global_count_items.len(), 3);
        for item in &tabs[&12] {
            assert_eq!(item.state, ScratchpadDisplayState::Closed);
            assert_eq!(item.tab_id, Some(12));
            assert_eq!(item.tab_position, Some(2));
            assert_eq!(item.pane_id, None);
            assert!(!item.is_focused);
            assert!(!item.is_mru);
        }
        assert_eq!(snapshot.current_items[0].tab_id, None);

        let inactive_ctx = ScratchpadContext {
            current_tab_id: Some(11),
            current_tab_position: 1,
            ..ctx
        };
        assert_eq!(manager.publication_snapshot(&inactive_ctx).1, tabs);
    }

    #[test]
    fn publication_rejects_moved_exited_held_and_plugin_panes() {
        let mut manager = ScratchpadManager::new(HashMap::from([
            ("term".to_string(), make_config(None)),
            ("notes".to_string(), make_config(None)),
        ]));
        manager.panes = HashMap::from([
            ("term".to_string(), HashMap::from([(10, 42), (11, 43)])),
            ("notes".to_string(), HashMap::from([(10, 44), (11, 45)])),
        ]);
        let manifest = HashMap::from([
            (
                0,
                vec![PaneInfo {
                    id: 44,
                    is_held: true,
                    ..Default::default()
                }],
            ),
            (
                1,
                vec![
                    make_pane(42, false, true),
                    PaneInfo {
                        id: 43,
                        exited: true,
                        ..Default::default()
                    },
                    PaneInfo {
                        id: 45,
                        is_plugin: true,
                        ..Default::default()
                    },
                ],
            ),
        ]);
        let positions = HashMap::from([(10, 0), (11, 1)]);
        let (_, tabs) = manager.publication_snapshot(&make_context(&manifest, &positions));
        for (&tab_id, items) in &tabs {
            for item in items {
                assert_eq!(item.state, ScratchpadDisplayState::Closed);
                assert_eq!(item.tab_id, Some(tab_id));
                assert_eq!(item.pane_id, None);
                assert!(!item.is_focused);
            }
        }
    }

    #[test]
    fn publication_uses_authoritative_tabs_even_when_empty_or_reordered() {
        let mut manager =
            ScratchpadManager::new(HashMap::from([("term".to_string(), make_config(None))]));
        manager.panes = HashMap::from([("term".to_string(), HashMap::from([(10, 42)]))]);
        let manifest = HashMap::from([(3, vec![make_pane(42, false, false)])]);
        let positions = HashMap::from([(10, 3)]);
        let (_, tabs) = manager.publication_snapshot(&make_context(&manifest, &positions));
        assert_eq!(tabs[&10][0].tab_position, Some(3));
        assert_eq!(tabs[&10][0].pane_id, Some(42));

        let positions = HashMap::new();
        let (_, tabs) = manager.publication_snapshot(&make_context(&manifest, &positions));
        assert!(tabs.is_empty());

        let manager = ScratchpadManager::new(HashMap::new());
        let positions = HashMap::from([(12, 2)]);
        let (_, tabs) = manager.publication_snapshot(&make_context(&manifest, &positions));
        assert_eq!(tabs, BTreeMap::from([(12, Vec::new())]));
    }

    #[test]
    fn current_snapshot_reports_closed_without_current_tab_pane() {
        let manager =
            ScratchpadManager::new(HashMap::from([("term".to_string(), make_config(None))]));
        let manifest = HashMap::new();
        let positions = HashMap::from([(10, 0)]);

        let snapshot = manager.status_snapshot(&make_context(&manifest, &positions));

        assert_eq!(snapshot.current_items[0].name, "term");
        assert_eq!(
            snapshot.current_items[0].state,
            ScratchpadDisplayState::Closed
        );
        assert_eq!(snapshot.current_items[0].title, "term");
        assert!(!snapshot.current_items[0].is_mru);
    }

    #[test]
    fn snapshot_reports_current_visible_and_focused() {
        let mut manager = ScratchpadManager::new(HashMap::from([(
            "term".to_string(),
            make_config(Some("Terminal")),
        )]));
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(10, 42);
        let manifest = HashMap::from([(0, vec![make_pane(42, false, true)])]);
        let positions = HashMap::from([(10, 0)]);

        let snapshot = manager.status_snapshot(&make_context(&manifest, &positions));

        assert_eq!(
            snapshot.current_items[0].state,
            ScratchpadDisplayState::Visible
        );
        assert!(snapshot.current_items[0].is_focused);
        assert!(snapshot.current_items[0].is_mru);
        assert_eq!(snapshot.current_items[0].pane_id, Some(42));
        assert_eq!(snapshot.current_items[0].title, "Terminal");
    }

    #[test]
    fn global_snapshot_prefers_focused_visible_instance() {
        let mut manager =
            ScratchpadManager::new(HashMap::from([("term".to_string(), make_config(None))]));
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(10, 42);
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(11, 43);
        let manifest = HashMap::from([
            (0, vec![make_pane(42, false, false)]),
            (1, vec![make_pane(43, false, true)]),
        ]);
        let positions = HashMap::from([(10, 0), (11, 1)]);

        let snapshot = manager.status_snapshot(&make_context(&manifest, &positions));

        assert_eq!(
            snapshot.global_items[0].state,
            ScratchpadDisplayState::Visible
        );
        assert_eq!(snapshot.global_items[0].pane_id, Some(43));
        assert!(snapshot.global_items[0].is_focused);
        assert_eq!(snapshot.global_count_items.len(), 2);
        assert_eq!(
            snapshot
                .global_count_items
                .iter()
                .filter(|item| item.state == ScratchpadDisplayState::Visible)
                .count(),
            2
        );
    }

    #[test]
    fn current_mru_matches_toggle_target_and_tracks_focus_changes() {
        let mut manager = ScratchpadManager::new(HashMap::from([
            ("term".to_string(), make_config(None)),
            ("notes".to_string(), make_config(None)),
        ]));
        manager.panes = HashMap::from([
            ("term".to_string(), HashMap::from([(10, 42)])),
            ("notes".to_string(), HashMap::from([(10, 43), (11, 44)])),
        ]);
        manager.focus_times = HashMap::from([
            ("term".to_string(), HashMap::from([(10, 2)])),
            ("notes".to_string(), HashMap::from([(10, 1), (11, 3)])),
        ]);
        manager.focus_counter = 3;
        let mut manifest = HashMap::from([(
            0,
            vec![make_pane(42, true, false), make_pane(43, true, false)],
        )]);
        let positions = HashMap::from([(10, 0), (11, 1)]);
        let ctx = make_context(&manifest, &positions);
        let snapshot = manager.status_snapshot(&ctx);
        let mru = snapshot
            .current_items
            .iter()
            .find(|item| item.is_mru)
            .unwrap();
        assert_eq!(mru.name, "term");
        assert_eq!(mru.state, ScratchpadDisplayState::Hidden);
        assert_eq!(
            Some(mru.name.clone()),
            manager.get_last_focused_on_current_tab(&ctx)
        );
        assert!(snapshot.global_items.iter().all(|item| !item.is_mru));

        manifest.get_mut(&0).unwrap()[1] = make_pane(43, false, true);
        let ctx = make_context(&manifest, &positions);
        let snapshot = manager.status_snapshot(&ctx);
        assert!(snapshot.current_items[0].is_focused);
        assert!(snapshot.current_items[0].is_mru);
        assert!(!snapshot.current_items[1].is_mru);
        manager.update_focus_tracking(&ctx);

        manifest.get_mut(&0).unwrap()[1] = make_pane(43, true, false);
        let snapshot = manager.status_snapshot(&make_context(&manifest, &positions));
        assert!(!snapshot.current_items[0].is_focused);
        assert!(snapshot.current_items[0].is_mru);
        assert!(!snapshot.current_items[1].is_mru);
    }

    #[test]
    fn global_snapshot_is_hidden_when_only_hidden_instances_exist() {
        let mut manager =
            ScratchpadManager::new(HashMap::from([("term".to_string(), make_config(None))]));
        manager
            .panes
            .entry("term".to_string())
            .or_default()
            .insert(10, 42);
        let manifest = HashMap::from([(0, vec![make_pane(42, true, false)])]);
        let positions = HashMap::from([(10, 0)]);

        let snapshot = manager.status_snapshot(&make_context(&manifest, &positions));

        assert_eq!(
            snapshot.global_items[0].state,
            ScratchpadDisplayState::Hidden
        );
        assert_eq!(snapshot.global_items[0].pane_id, Some(42));
    }
}
