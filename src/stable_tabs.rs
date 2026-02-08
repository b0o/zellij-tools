use std::collections::{HashMap, HashSet};
use zellij_tile::prelude::PaneInfo;

/// A stable tab identifier that doesn't change when tabs are reordered.
/// We use a monotonically increasing counter assigned when we first see a tab.
pub type StableTabId = u64;

/// Tracks stable tab IDs across tab reorders.
///
/// Zellij identifies tabs by position, which changes when tabs are reordered.
/// This tracker assigns stable IDs by finding "reference panes" (tiled panes)
/// on each tab. The stable ID follows the reference pane, not the position.
#[derive(Default)]
pub struct StableTabTracker {
    next_id: StableTabId,
    /// reference_pane_id -> stable_tab_id (tiled panes that identify tabs)
    reference_pane_to_tab: HashMap<u32, StableTabId>,
    /// stable_tab_id -> current position (updated on each update)
    pub stable_tab_to_position: HashMap<StableTabId, usize>,
    /// current position -> stable_tab_id (reverse lookup, rebuilt at end of update)
    pub position_to_stable: HashMap<usize, StableTabId>,
}

impl StableTabTracker {
    /// Get stable ID for a tab position, if known
    pub fn get_stable_id(&self, position: usize) -> Option<StableTabId> {
        self.position_to_stable.get(&position).copied()
    }

    /// Update mappings based on current pane manifest.
    /// Returns set of orphaned stable IDs (tabs that were closed).
    ///
    /// The algorithm handles these edge cases:
    /// 1. Reference pane moved to different tab position -> stable ID follows the pane
    /// 2. Reference pane closed -> pick new reference pane from remaining tiled panes
    /// 3. Tab closed entirely -> stable ID becomes orphaned
    pub fn update(
        &mut self,
        pane_manifest: &HashMap<usize, Vec<PaneInfo>>,
    ) -> HashSet<StableTabId> {
        // Build a map of pane_id -> current tab position for all tiled panes
        let pane_to_current_tab: HashMap<u32, usize> = pane_manifest
            .iter()
            .flat_map(|(&tab_pos, panes)| {
                panes
                    .iter()
                    .filter(|p| !p.is_floating && !p.is_plugin)
                    .map(move |p| (p.id, tab_pos))
            })
            .collect();

        // Clean up reference panes that no longer exist (were closed)
        let existing_pane_ids: HashSet<u32> = pane_to_current_tab.keys().copied().collect();
        let closed_refs: Vec<(u32, StableTabId)> = self
            .reference_pane_to_tab
            .iter()
            .filter(|(pane_id, _)| !existing_pane_ids.contains(pane_id))
            .map(|(&pane_id, &stable_id)| (pane_id, stable_id))
            .collect();

        for (pane_id, _stable_id) in &closed_refs {
            self.reference_pane_to_tab.remove(pane_id);
        }

        // Rebuild stable_tab_to_position based on where reference panes are NOW
        // (stable ID follows the reference pane, even if position changed)
        self.stable_tab_to_position.clear();
        self.position_to_stable.clear();
        for (&pane_id, &stable_id) in &self.reference_pane_to_tab {
            if let Some(&tab_position) = pane_to_current_tab.get(&pane_id) {
                self.stable_tab_to_position.insert(stable_id, tab_position);
                self.position_to_stable.insert(tab_position, stable_id);
            }
        }

        // For closed reference panes, try to find a replacement on an unassigned tab
        for (_closed_pane_id, stable_id) in &closed_refs {
            // Skip if this stable ID already got reassigned
            if self.stable_tab_to_position.contains_key(stable_id) {
                continue;
            }

            // Find tabs that don't have a stable ID yet
            for (&tab_position, panes) in pane_manifest {
                // Skip if this position already has a stable ID
                if self.position_to_stable.contains_key(&tab_position) {
                    continue;
                }

                // Get tiled panes on this tab
                let tiled_panes: Vec<u32> = panes
                    .iter()
                    .filter(|p| !p.is_floating && !p.is_plugin)
                    .map(|p| p.id)
                    .collect();

                if let Some(&new_ref_pane) = tiled_panes.first() {
                    // Assign this orphaned stable ID to this tab with a new reference pane
                    self.reference_pane_to_tab.insert(new_ref_pane, *stable_id);
                    self.stable_tab_to_position.insert(*stable_id, tab_position);
                    self.position_to_stable.insert(tab_position, *stable_id);
                    break;
                }
            }
        }

        // Collect orphaned stable IDs (those with closed refs that couldn't be reassigned)
        let orphaned: HashSet<StableTabId> = closed_refs
            .iter()
            .map(|(_, stable_id)| *stable_id)
            .filter(|stable_id| !self.stable_tab_to_position.contains_key(stable_id))
            .collect();

        // Assign new stable IDs to any tabs that still don't have one
        for (&tab_position, panes) in pane_manifest {
            // Skip if this position already has a stable ID
            if self.position_to_stable.contains_key(&tab_position) {
                continue;
            }

            // Get tiled panes on this tab
            let tiled_panes: Vec<u32> = panes
                .iter()
                .filter(|p| !p.is_floating && !p.is_plugin)
                .map(|p| p.id)
                .collect();

            if tiled_panes.is_empty() {
                continue;
            }

            let new_id = self.next_id;
            self.next_id += 1;
            self.reference_pane_to_tab.insert(tiled_panes[0], new_id);
            self.stable_tab_to_position.insert(new_id, tab_position);
            self.position_to_stable.insert(tab_position, new_id);
        }

        orphaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pane(id: u32, is_floating: bool, is_plugin: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_floating,
            is_plugin,
            ..Default::default()
        }
    }

    fn make_tiled_pane(id: u32) -> PaneInfo {
        make_pane(id, false, false)
    }

    fn make_floating_pane(id: u32) -> PaneInfo {
        make_pane(id, true, false)
    }

    #[test]
    fn new_tab_gets_stable_id() {
        let mut tracker = StableTabTracker::default();
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_tiled_pane(100)]);

        let orphaned = tracker.update(&manifest);

        assert!(orphaned.is_empty());
        assert_eq!(tracker.get_stable_id(0), Some(0));
    }

    #[test]
    fn multiple_tabs_get_different_ids() {
        let mut tracker = StableTabTracker::default();
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_tiled_pane(100)]);
        manifest.insert(1, vec![make_tiled_pane(200)]);

        tracker.update(&manifest);

        let id0 = tracker.get_stable_id(0);
        let id1 = tracker.get_stable_id(1);
        assert!(id0.is_some());
        assert!(id1.is_some());
        assert_ne!(id0, id1);
    }

    #[test]
    fn stable_id_follows_pane_when_tabs_reorder() {
        let mut tracker = StableTabTracker::default();

        // Initial: tab 0 has pane 100, tab 1 has pane 200
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_tiled_pane(100)]);
        manifest.insert(1, vec![make_tiled_pane(200)]);
        tracker.update(&manifest);

        let original_id_for_pane100 = tracker.get_stable_id(0).unwrap();

        // Tabs reordered: pane 100 now at position 1, pane 200 at position 0
        manifest.clear();
        manifest.insert(0, vec![make_tiled_pane(200)]);
        manifest.insert(1, vec![make_tiled_pane(100)]);
        tracker.update(&manifest);

        // Stable ID should follow the pane, not the position
        assert_eq!(tracker.get_stable_id(1), Some(original_id_for_pane100));
    }

    #[test]
    fn floating_panes_ignored_for_reference() {
        let mut tracker = StableTabTracker::default();
        let mut manifest = HashMap::new();
        // Tab with only floating panes should not get a stable ID
        manifest.insert(0, vec![make_floating_pane(100)]);

        tracker.update(&manifest);

        assert_eq!(tracker.get_stable_id(0), None);
    }

    #[test]
    fn tab_closed_returns_orphaned_id() {
        let mut tracker = StableTabTracker::default();

        // Two tabs
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_tiled_pane(100)]);
        manifest.insert(1, vec![make_tiled_pane(200)]);
        tracker.update(&manifest);

        let id_for_tab1 = tracker.get_stable_id(1).unwrap();

        // Close tab 1 (remove pane 200)
        manifest.remove(&1);
        let orphaned = tracker.update(&manifest);

        assert!(orphaned.contains(&id_for_tab1));
    }

    #[test]
    fn reverse_map_consistent_after_reorder() {
        let mut tracker = StableTabTracker::default();

        // Initial: 3 tabs
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_tiled_pane(100)]);
        manifest.insert(1, vec![make_tiled_pane(200)]);
        manifest.insert(2, vec![make_tiled_pane(300)]);
        tracker.update(&manifest);

        let id0 = tracker.get_stable_id(0).unwrap();
        let id1 = tracker.get_stable_id(1).unwrap();
        let id2 = tracker.get_stable_id(2).unwrap();

        // Reorder: reverse the tab positions
        manifest.clear();
        manifest.insert(0, vec![make_tiled_pane(300)]);
        manifest.insert(1, vec![make_tiled_pane(200)]);
        manifest.insert(2, vec![make_tiled_pane(100)]);
        tracker.update(&manifest);

        // Stable IDs follow panes, not positions
        assert_eq!(tracker.get_stable_id(0), Some(id2));
        assert_eq!(tracker.get_stable_id(1), Some(id1));
        assert_eq!(tracker.get_stable_id(2), Some(id0));

        // Verify the reverse map is consistent with forward map
        for (&stable_id, &position) in &tracker.stable_tab_to_position {
            assert_eq!(
                tracker.position_to_stable.get(&position),
                Some(&stable_id),
                "position_to_stable inconsistent for position {} (expected stable_id {})",
                position,
                stable_id
            );
        }
        for (&position, &stable_id) in &tracker.position_to_stable {
            assert_eq!(
                tracker.stable_tab_to_position.get(&stable_id),
                Some(&position),
                "stable_tab_to_position inconsistent for stable_id {} (expected position {})",
                stable_id,
                position
            );
        }
    }

    #[test]
    fn reference_pane_closed_picks_new_reference() {
        let mut tracker = StableTabTracker::default();

        // Tab with two tiled panes
        let mut manifest = HashMap::new();
        manifest.insert(0, vec![make_tiled_pane(100), make_tiled_pane(101)]);
        tracker.update(&manifest);

        let original_stable_id = tracker.get_stable_id(0).unwrap();

        // Close the reference pane (100), but 101 remains
        manifest.insert(0, vec![make_tiled_pane(101)]);
        let orphaned = tracker.update(&manifest);

        // Should not be orphaned - picked new reference
        assert!(orphaned.is_empty());
        // Same stable ID for the tab
        assert_eq!(tracker.get_stable_id(0), Some(original_stable_id));
    }
}
