use std::collections::{HashMap, HashSet};

use zellij_tile::prelude::{PaneId, PaneInfo};

/// A pane manifest keyed by Zellij's tab position.
pub type RawPaneManifest = HashMap<usize, Vec<PaneInfo>>;

/// Tracks ownership of typed panes by Zellij's stable native tab IDs.
///
/// Zellij reports panes by tab position and reports tab IDs separately.  Tab
/// positions can change between those two updates, so pane ownership is kept
/// by `PaneId` and is only changed by a position when that change is safe to
/// observe.  The tracker deliberately does not emit events or mutate Zellij.
#[derive(Debug, Default)]
pub struct PaneTopology {
    raw_manifest: RawPaneManifest,
    owners: HashMap<PaneId, usize>,
    pending_positions: HashMap<PaneId, usize>,
    position_to_tab_id: HashMap<usize, usize>,
    live_tab_ids: HashSet<usize>,
    /// True when the last applied tab mapping does not yet agree with the
    /// latest pane layout.  This is the event-order boundary: a pane update
    /// that arrives first must wait, while a pane update after TabUpdate can
    /// reconcile immediately.
    tab_update_unmatched: bool,
}

impl PaneTopology {
    /// Ingest a pane update.
    pub fn ingest(
        &mut self,
        manifest: &RawPaneManifest,
        position_to_tab_id: &HashMap<usize, usize>,
        live_tab_ids: &HashSet<usize>,
    ) {
        let manifest = manifest.clone();

        let previous_positions = pane_positions(&self.raw_manifest);
        let current_positions = pane_positions(&manifest);
        let previous_keys: HashSet<_> = previous_positions.keys().copied().collect();
        let current_keys: HashSet<_> = current_positions.keys().copied().collect();
        let previous_pending = std::mem::take(&mut self.pending_positions);

        let shift = classify_shift(&previous_positions, &current_positions, &self.owners);
        let current_group_owners = current_group_owners(&current_positions, &self.owners);
        let can_reconcile_shift_now = self.tab_update_unmatched;
        let has_new_panes = current_keys
            .iter()
            .any(|pane_id| !previous_keys.contains(pane_id));
        let has_merged_group = current_group_owners.values().any(|owners| owners.len() > 1);
        let ambiguous_shift = shift.is_topology_shift
            || (!shift.shifted_owners.is_empty() && (has_new_panes || has_merged_group));
        let ambiguous_replacement = !shift.disappeared_owners.is_empty() && has_new_panes;
        let mut next_pending = HashMap::new();

        for (&pane_id, &position) in &current_positions {
            let previous_owner = self.owners.get(&pane_id).copied();
            let was_pending = previous_pending.contains_key(&pane_id);
            let is_new = !previous_keys.contains(&pane_id);
            let moved = previous_positions
                .get(&pane_id)
                .is_some_and(|previous_position| *previous_position != position);
            let group_owner = current_group_owners.get(&position).and_then(single_owner);
            let mapped_owner = mapped_owner(position, position_to_tab_id, live_tab_ids);

            let owner = if was_pending {
                // A pending position is intentionally not resolved by another
                // PaneUpdate.  It needs the TabUpdate that describes the new
                // position-to-ID mapping.
                previous_owner
            } else if ambiguous_replacement && is_new && !can_reconcile_shift_now {
                group_owner
            } else if ambiguous_shift {
                if is_new {
                    group_owner
                        .or_else(|| can_reconcile_shift_now.then_some(mapped_owner).flatten())
                } else if can_reconcile_shift_now {
                    mapped_owner.or(previous_owner)
                } else {
                    previous_owner.or(group_owner).or(mapped_owner)
                }
            } else if is_new {
                mapped_owner
            } else if moved {
                // A pane moving by itself is not evidence that native tabs
                // moved.  Prefer the current group identity when available;
                // this also works while the tab map is still one event behind.
                current_group_owner_except(position, pane_id, &current_positions, &self.owners)
                    .or(mapped_owner)
                    .or(previous_owner)
            } else {
                // A TabUpdate-first reorder can be followed by a stale
                // PaneUpdate.  Position-stable panes must keep their owner.
                previous_owner.or(mapped_owner)
            };

            if let Some(owner) = owner {
                self.owners.insert(pane_id, owner);
            } else {
                self.owners.remove(&pane_id);
            }

            let needs_tab_update = was_pending
                || (ambiguous_shift && !can_reconcile_shift_now)
                || (ambiguous_replacement && is_new && !can_reconcile_shift_now)
                || (is_new && mapped_owner.is_none())
                || (moved && mapped_owner.is_none() && group_owner.is_none());

            if needs_tab_update {
                next_pending.insert(pane_id, position);
            }
        }

        // A PaneUpdate is authoritative for pane existence.  This removes a
        // pane only after it disappears from a real pane snapshot, not when a
        // TabUpdate merely removes its native tab ID from the live set.
        self.owners
            .retain(|pane_id, _| current_keys.contains(pane_id));
        next_pending.retain(|pane_id, _| current_keys.contains(pane_id));

        self.raw_manifest = manifest;
        self.pending_positions = next_pending;

        // A deferred pane update leaves the mapping unmatched.  Otherwise,
        // compare against the mapping supplied with this event so a
        // TabUpdate-first snapshot can be recognized as reconciled.
        self.tab_update_unmatched = !self.pending_positions.is_empty()
            || !owners_match_mapping(
                &current_positions,
                &self.owners,
                position_to_tab_id,
                live_tab_ids,
            );

        // These are deliberately not installed as the applied tab state:
        // when PaneUpdate arrives first, its map can still be the old map.
        // The applied maps change only in `apply_tab_update`.
    }

    /// Apply a tab update and resolve pane positions observed by the previous
    /// pane update.
    pub fn apply_tab_update(
        &mut self,
        position_to_tab_id: &HashMap<usize, usize>,
        live_tab_ids: &HashSet<usize>,
    ) {
        self.position_to_tab_id = position_to_tab_id.clone();
        self.live_tab_ids = live_tab_ids.clone();

        let current_positions = pane_positions(&self.raw_manifest);
        let pending = std::mem::take(&mut self.pending_positions);
        for (pane_id, observed_position) in pending {
            if !current_positions.contains_key(&pane_id) {
                continue;
            }
            if let Some(owner) = mapped_owner(observed_position, position_to_tab_id, live_tab_ids) {
                self.owners.insert(pane_id, owner);
            } else {
                self.pending_positions.insert(pane_id, observed_position);
            }
        }

        self.tab_update_unmatched = !self.pending_positions.is_empty()
            || !owners_match_mapping(
                &current_positions,
                &self.owners,
                position_to_tab_id,
                live_tab_ids,
            );
    }

    /// The latest raw position-keyed manifest, including panes whose owner is
    /// still pending.
    pub fn raw_manifest(&self) -> &RawPaneManifest {
        &self.raw_manifest
    }

    /// Group the complete raw `PaneInfo` values by native tab ID.
    ///
    /// Only panes whose owner is in `live_tab_ids` are returned.  Tabs without
    /// a pane are not added to the result.
    pub fn panes_by_tab_id(&self, live_tab_ids: &HashSet<usize>) -> HashMap<usize, Vec<PaneInfo>> {
        let mut panes_by_tab_id: HashMap<usize, Vec<PaneInfo>> = HashMap::new();

        let mut groups: Vec<_> = self.raw_manifest.iter().collect();
        groups.sort_unstable_by_key(|(position, _)| **position);
        for (_, panes) in groups {
            for pane in panes {
                let pane_id = pane_id(pane);
                let Some(&tab_id) = self.owners.get(&pane_id) else {
                    continue;
                };
                if live_tab_ids.contains(&tab_id) {
                    panes_by_tab_id
                        .entry(tab_id)
                        .or_default()
                        .push(pane.clone());
                }
            }
        }

        panes_by_tab_id
    }

    /// Group panes using the live tab set from the last applied TabUpdate.
    pub fn live_panes_by_tab_id(&self) -> HashMap<usize, Vec<PaneInfo>> {
        self.panes_by_tab_id(&self.live_tab_ids)
    }

    /// Return the currently retained owner for a typed pane.
    pub fn pane_owner(&self, pane_id: PaneId) -> Option<usize> {
        self.owners.get(&pane_id).copied()
    }

    /// Whether one or more observed pane positions still need a TabUpdate.
    pub fn has_pending(&self) -> bool {
        !self.pending_positions.is_empty()
    }
}

#[derive(Debug, Default)]
struct ShiftClassification {
    shifted_owners: HashSet<usize>,
    disappeared_owners: HashSet<usize>,
    split_owners: HashSet<usize>,
    is_topology_shift: bool,
}

fn pane_id(pane: &PaneInfo) -> PaneId {
    if pane.is_plugin {
        PaneId::Plugin(pane.id)
    } else {
        PaneId::Terminal(pane.id)
    }
}

fn pane_positions(manifest: &RawPaneManifest) -> HashMap<PaneId, usize> {
    let mut positions = HashMap::new();
    let mut entries: Vec<_> = manifest.iter().collect();
    entries.sort_unstable_by_key(|(position, _)| **position);
    for (&position, panes) in entries {
        for pane in panes {
            positions.entry(pane_id(pane)).or_insert(position);
        }
    }
    positions
}

fn classify_shift(
    previous_positions: &HashMap<PaneId, usize>,
    current_positions: &HashMap<PaneId, usize>,
    owners: &HashMap<PaneId, usize>,
) -> ShiftClassification {
    let mut prior_groups: HashMap<usize, (HashSet<usize>, HashSet<PaneId>)> = HashMap::new();
    for (&pane_id, &position) in previous_positions {
        let Some(&owner) = owners.get(&pane_id) else {
            continue;
        };
        let (positions, panes) = prior_groups.entry(owner).or_default();
        positions.insert(position);
        panes.insert(pane_id);
    }

    let mut classification = ShiftClassification::default();
    for (&owner, (prior_positions, panes)) in &prior_groups {
        if prior_positions.len() != 1 {
            continue;
        }
        let surviving_positions: HashSet<_> = panes
            .iter()
            .filter_map(|pane_id| current_positions.get(pane_id).copied())
            .collect();
        if surviving_positions.is_empty() {
            classification.disappeared_owners.insert(owner);
        } else if surviving_positions.len() > 1 {
            classification.split_owners.insert(owner);
        } else if surviving_positions != *prior_positions {
            classification.shifted_owners.insert(owner);
        }
    }

    classification.is_topology_shift = classification.shifted_owners.len() >= 2
        || (!classification.shifted_owners.is_empty()
            && (!classification.disappeared_owners.is_empty()
                || !classification.split_owners.is_empty()));
    classification
}

fn current_group_owners(
    current_positions: &HashMap<PaneId, usize>,
    owners: &HashMap<PaneId, usize>,
) -> HashMap<usize, HashSet<usize>> {
    let mut group_owners: HashMap<usize, HashSet<usize>> = HashMap::new();
    for (&pane_id, &position) in current_positions {
        if let Some(&owner) = owners.get(&pane_id) {
            group_owners.entry(position).or_default().insert(owner);
        }
    }
    group_owners
}

fn current_group_owner_except(
    position: usize,
    excluded_pane_id: PaneId,
    current_positions: &HashMap<PaneId, usize>,
    owners: &HashMap<PaneId, usize>,
) -> Option<usize> {
    let group_owners: HashSet<_> = current_positions
        .iter()
        .filter(|(pane_id, pane_position)| {
            **pane_id != excluded_pane_id && **pane_position == position
        })
        .filter_map(|(pane_id, _)| owners.get(pane_id).copied())
        .collect();
    single_owner(&group_owners)
}

fn single_owner(owners: &HashSet<usize>) -> Option<usize> {
    (owners.len() == 1)
        .then(|| owners.iter().next().copied())
        .flatten()
}

fn mapped_owner(
    position: usize,
    position_to_tab_id: &HashMap<usize, usize>,
    live_tab_ids: &HashSet<usize>,
) -> Option<usize> {
    position_to_tab_id
        .get(&position)
        .copied()
        .filter(|tab_id| live_tab_ids.contains(tab_id))
}

fn owners_match_mapping(
    positions: &HashMap<PaneId, usize>,
    owners: &HashMap<PaneId, usize>,
    position_to_tab_id: &HashMap<usize, usize>,
    live_tab_ids: &HashSet<usize>,
) -> bool {
    positions.iter().all(|(pane_id, &position)| {
        mapped_owner(position, position_to_tab_id, live_tab_ids)
            .is_none_or(|mapped| owners.get(pane_id) == Some(&mapped))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: u32, is_plugin: bool) -> PaneInfo {
        PaneInfo {
            id,
            is_plugin,
            title: format!("pane-{id}"),
            ..Default::default()
        }
    }

    fn terminal(id: u32) -> PaneInfo {
        pane(id, false)
    }

    fn plugin(id: u32) -> PaneInfo {
        pane(id, true)
    }

    fn manifest(groups: &[(usize, Vec<PaneInfo>)]) -> RawPaneManifest {
        groups.iter().cloned().collect()
    }

    fn tabs(entries: &[(usize, usize)]) -> (HashMap<usize, usize>, HashSet<usize>) {
        let position_to_tab_id = entries.iter().copied().collect();
        let live_tab_ids = entries.iter().map(|(_, tab_id)| *tab_id).collect();
        (position_to_tab_id, live_tab_ids)
    }

    #[test]
    fn pane_update_before_tab_update_preserves_reordered_groups() {
        let old = manifest(&[(0, vec![terminal(10)]), (1, vec![terminal(20)])]);
        let reordered = manifest(&[(0, vec![terminal(20)]), (1, vec![terminal(10)])]);
        let (old_positions, old_live) = tabs(&[(0, 100), (1, 200)]);
        let (new_positions, new_live) = tabs(&[(0, 200), (1, 100)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.ingest(&reordered, &old_positions, &old_live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(topology.has_pending());

        topology.apply_tab_update(&new_positions, &new_live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(!topology.has_pending());
        assert_eq!(topology.panes_by_tab_id(&new_live)[&100][0].id, 10);
        assert_eq!(topology.panes_by_tab_id(&new_live)[&200][0].id, 20);
    }

    #[test]
    fn tab_update_before_pane_update_reconciles_only_the_real_snapshot() {
        let old = manifest(&[(0, vec![terminal(10)]), (1, vec![terminal(20)])]);
        let reordered = manifest(&[(0, vec![terminal(20)]), (1, vec![terminal(10)])]);
        let (old_positions, old_live) = tabs(&[(0, 100), (1, 200)]);
        let (new_positions, new_live) = tabs(&[(0, 200), (1, 100)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.apply_tab_update(&new_positions, &new_live);

        // This is the old pane snapshot arriving after the TabUpdate.
        topology.ingest(&old, &new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(!topology.has_pending());

        topology.ingest(&reordered, &new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(!topology.has_pending());
    }

    #[test]
    fn closing_an_earlier_tab_filters_the_closed_owner_until_panes_update() {
        let old = manifest(&[
            (0, vec![terminal(10)]),
            (1, vec![terminal(20)]),
            (2, vec![terminal(30)]),
        ]);
        let after_close = manifest(&[(0, vec![terminal(20)]), (1, vec![terminal(30)])]);
        let (old_positions, old_live) = tabs(&[(0, 100), (1, 200), (2, 300)]);
        let (new_positions, new_live) = tabs(&[(0, 200), (1, 300)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.apply_tab_update(&new_positions, &new_live);

        let before_pane_update = topology.panes_by_tab_id(&new_live);
        assert!(!before_pane_update
            .values()
            .flatten()
            .any(|pane| pane.id == 10));
        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));

        topology.ingest(&after_close, &new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), None);
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert_eq!(topology.pane_owner(PaneId::Terminal(30)), Some(300));
        assert!(!topology.has_pending());
    }

    #[test]
    fn one_shifted_group_and_one_disappeared_group_are_a_topology_shift() {
        let old = manifest(&[(0, vec![terminal(10)]), (1, vec![terminal(20)])]);
        let after_close = manifest(&[(0, vec![terminal(20)])]);
        let (old_positions, old_live) = tabs(&[(0, 100), (1, 200)]);
        let (new_positions, new_live) = tabs(&[(0, 200)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.ingest(&after_close, &old_positions, &old_live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(topology.has_pending());

        topology.apply_tab_update(&new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(!topology.has_pending());
    }

    #[test]
    fn isolated_pane_move_adopts_destination_without_tab_update() {
        let old = manifest(&[
            (0, vec![terminal(10), terminal(11)]),
            (1, vec![terminal(20)]),
        ]);
        let moved = manifest(&[
            (0, vec![terminal(10)]),
            (1, vec![terminal(20), terminal(11)]),
        ]);
        let (positions, live) = tabs(&[(0, 100), (1, 200)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &positions, &live);
        topology.ingest(&moved, &positions, &live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));
        assert_eq!(topology.pane_owner(PaneId::Terminal(11)), Some(200));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(!topology.has_pending());
    }

    #[test]
    fn sole_pane_move_to_empty_existing_tab_adopts_destination() {
        let old = manifest(&[(0, vec![terminal(10)]), (1, vec![])]);
        let moved = manifest(&[(0, vec![]), (1, vec![terminal(10)])]);
        let (positions, live) = tabs(&[(0, 100), (1, 200)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &positions, &live);
        topology.ingest(&moved, &positions, &live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(200));
        assert_eq!(topology.panes_by_tab_id(&live)[&200][0].id, 10);
        assert!(!topology.has_pending());
    }

    #[test]
    fn new_tab_pane_first_stays_pending_until_tab_update() {
        let old = manifest(&[(0, vec![terminal(10)])]);
        let with_new_tab = manifest(&[(0, vec![terminal(10)]), (1, vec![terminal(20)])]);
        let (old_positions, old_live) = tabs(&[(0, 100)]);
        let (new_positions, new_live) = tabs(&[(0, 100), (1, 200)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.ingest(&with_new_tab, &old_positions, &old_live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), None);
        assert!(topology.has_pending());
        assert_eq!(topology.raw_manifest()[&1][0].id, 20);

        topology.apply_tab_update(&new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(!topology.has_pending());
    }

    #[test]
    fn terminal_and_plugin_panes_with_the_same_number_are_independent() {
        let initial = manifest(&[
            (0, vec![terminal(7), plugin(7)]),
            (1, vec![terminal(8), plugin(8)]),
        ]);
        let reordered = manifest(&[
            (0, vec![terminal(8), plugin(8)]),
            (1, vec![terminal(7), plugin(7)]),
        ]);
        let (old_positions, old_live) = tabs(&[(0, 10), (1, 20)]);
        let (new_positions, new_live) = tabs(&[(0, 20), (1, 10)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&initial, &old_positions, &old_live);
        topology.ingest(&reordered, &old_positions, &old_live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(7)), Some(10));
        assert_eq!(topology.pane_owner(PaneId::Plugin(7)), Some(10));
        assert_eq!(topology.pane_owner(PaneId::Terminal(8)), Some(20));
        assert_eq!(topology.pane_owner(PaneId::Plugin(8)), Some(20));

        topology.apply_tab_update(&new_positions, &new_live);
        let by_tab = topology.panes_by_tab_id(&new_live);
        assert_eq!(by_tab[&10].len(), 2);
        assert_eq!(by_tab[&20].len(), 2);
    }

    #[test]
    fn new_pane_in_shifted_group_inherits_an_unambiguous_owner() {
        let old = manifest(&[(0, vec![terminal(10)]), (1, vec![terminal(20)])]);
        let reordered_with_new = manifest(&[
            (0, vec![terminal(20), terminal(21)]),
            (1, vec![terminal(10)]),
        ]);
        let (old_positions, old_live) = tabs(&[(0, 100), (1, 200)]);
        let (new_positions, new_live) = tabs(&[(0, 200), (1, 100)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.ingest(&reordered_with_new, &old_positions, &old_live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(21)), Some(200));
        assert!(topology.has_pending());
        topology.apply_tab_update(&new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(21)), Some(200));
        assert!(!topology.has_pending());
        assert_eq!(topology.panes_by_tab_id(&new_live)[&200].len(), 2);
    }

    #[test]
    fn close_then_create_at_same_position_waits_for_native_tab_id() {
        let old = manifest(&[(0, vec![terminal(10)])]);
        let replacement = manifest(&[(0, vec![terminal(20)])]);
        let (old_positions, old_live) = tabs(&[(0, 100)]);
        let (new_positions, new_live) = tabs(&[(0, 200)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.ingest(&replacement, &old_positions, &old_live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), None);
        assert!(topology.has_pending());
        assert!(topology.panes_by_tab_id(&old_live).is_empty());

        topology.apply_tab_update(&new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert_eq!(topology.panes_by_tab_id(&new_live)[&200][0].id, 20);
    }

    #[test]
    fn tab_update_first_resolves_replacement_immediately() {
        let old = manifest(&[(0, vec![terminal(10)])]);
        let replacement = manifest(&[(0, vec![terminal(20)])]);
        let (old_positions, old_live) = tabs(&[(0, 100)]);
        let (new_positions, new_live) = tabs(&[(0, 200)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.apply_tab_update(&new_positions, &new_live);
        topology.ingest(&replacement, &new_positions, &new_live);

        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), None);
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert!(!topology.has_pending());
    }

    #[test]
    fn tab_insertion_does_not_assign_new_pane_through_stale_position() {
        let old = manifest(&[(0, vec![terminal(10)]), (1, vec![terminal(20)])]);
        let inserted = manifest(&[
            (0, vec![terminal(30)]),
            (1, vec![terminal(10)]),
            (2, vec![terminal(20)]),
        ]);
        let (old_positions, old_live) = tabs(&[(0, 100), (1, 200)]);
        let (new_positions, new_live) = tabs(&[(0, 300), (1, 100), (2, 200)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.ingest(&inserted, &old_positions, &old_live);

        assert_ne!(topology.pane_owner(PaneId::Terminal(30)), Some(100));
        assert!(topology.has_pending());

        topology.apply_tab_update(&new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
        assert_eq!(topology.pane_owner(PaneId::Terminal(30)), Some(300));
    }

    #[test]
    fn pane_move_concurrent_with_tab_reorder_resolves_from_new_mapping() {
        let old = manifest(&[
            (0, vec![terminal(10), terminal(11)]),
            (1, vec![terminal(20)]),
        ]);
        let reordered_and_moved = manifest(&[
            (0, vec![terminal(20), terminal(11)]),
            (1, vec![terminal(10)]),
        ]);
        let (old_positions, old_live) = tabs(&[(0, 100), (1, 200)]);
        let (new_positions, new_live) = tabs(&[(0, 200), (1, 100)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.ingest(&reordered_and_moved, &old_positions, &old_live);

        assert!(topology.has_pending());
        assert_eq!(topology.pane_owner(PaneId::Terminal(11)), Some(100));

        topology.apply_tab_update(&new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));
        assert_eq!(topology.pane_owner(PaneId::Terminal(11)), Some(200));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
    }

    #[test]
    fn sole_pane_merge_waits_for_tab_closure_mapping() {
        let old = manifest(&[(0, vec![terminal(10)]), (1, vec![terminal(20)])]);
        let merged = manifest(&[(0, vec![terminal(10), terminal(20)])]);
        let (old_positions, old_live) = tabs(&[(0, 100), (1, 200)]);
        let (new_positions, new_live) = tabs(&[(0, 200)]);

        let mut topology = PaneTopology::default();
        topology.ingest(&old, &old_positions, &old_live);
        topology.ingest(&merged, &old_positions, &old_live);

        assert!(topology.has_pending());
        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(100));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));

        topology.apply_tab_update(&new_positions, &new_live);
        assert_eq!(topology.pane_owner(PaneId::Terminal(10)), Some(200));
        assert_eq!(topology.pane_owner(PaneId::Terminal(20)), Some(200));
    }
}
