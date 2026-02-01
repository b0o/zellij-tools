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
}

impl StableTabTracker {
    /// Get stable ID for a tab position, if known
    pub fn get_stable_id(&self, position: usize) -> Option<StableTabId> {
        self.stable_tab_to_position
            .iter()
            .find(|(_, &pos)| pos == position)
            .map(|(&id, _)| id)
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
        for (&pane_id, &stable_id) in &self.reference_pane_to_tab {
            if let Some(&tab_position) = pane_to_current_tab.get(&pane_id) {
                self.stable_tab_to_position.insert(stable_id, tab_position);
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
                if self
                    .stable_tab_to_position
                    .values()
                    .any(|&pos| pos == tab_position)
                {
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
            if self
                .stable_tab_to_position
                .values()
                .any(|&pos| pos == tab_position)
            {
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
        }

        orphaned
    }
}
