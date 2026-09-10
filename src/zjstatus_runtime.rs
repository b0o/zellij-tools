//! Pure publication and timer state for zjstatus integration.
//!
//! Main must gate publication until its first authoritative tab snapshot. Thereafter
//! `tab_items` is the complete live native-ID set, including tabs with empty items;
//! an empty map means all tabs closed, not an unavailable snapshot.
//!
//! Scheduler times are elapsed durations from one main-owned `Instant` origin, not
//! the elapsed value carried by Zellij timer events. No host calls occur here.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use zellij_tile::prelude::PaneInfo;

use crate::pane_status::PaneStatusSnapshot;
use crate::scratchpad::{ScratchpadStatusItem, ScratchpadStatusSnapshot};
use crate::zjstatus::{
    render, render_pane_status, should_publish_empty, zjstatus_payload, zjstatus_tab_payload,
    OutputKey, OutputSource, ZjstatusConfig,
};

/// Update receiver discovery state. A changed set needs an immediate full
/// publication plus a delayed replay for receivers that are still initializing.
pub fn update_zjstatus_plugin_panes(
    previous: &mut HashSet<(usize, u32)>,
    pane_manifest: &HashMap<usize, Vec<PaneInfo>>,
) -> bool {
    let panes = pane_manifest
        .iter()
        .flat_map(|(&tab_position, panes)| {
            panes.iter().filter_map(move |pane| {
                let is_zjstatus = pane
                    .plugin_url
                    .as_deref()
                    .is_some_and(|url| url == "zjstatus" || url.contains("zjstatus"));
                (pane.is_plugin && is_zjstatus).then_some((tab_position, pane.id))
            })
        })
        .collect();
    if panes == *previous {
        return false;
    }
    *previous = panes;
    true
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PublishedKey {
    Global(String),
    Tab(usize, String),
}

impl PublishedKey {
    fn command(&self, value: &str) -> String {
        match self {
            Self::Global(name) => zjstatus_payload(name, value),
            Self::Tab(id, field) => zjstatus_tab_payload(*id, field, value),
        }
    }
}

/// Tracks submitted values, not delivery acknowledgements.
///
/// Removed tab fields remain retired for this producer's lifetime while their tab
/// stays live. History is bounded to owned fields on live IDs, not closed tabs;
/// repeated field renames on a long-lived tab can still grow that history. Restart
/// loses ownership history. Pane-status global clears survive for the producer's
/// lifetime; scratchpad global removal deliberately does not attempt a clear.
#[derive(Debug, Default)]
pub struct Publisher {
    published: BTreeMap<PublishedKey, String>,
    pane_status_globals: BTreeSet<PublishedKey>,
    retired: BTreeSet<PublishedKey>,
}

impl Publisher {
    /// Compute deterministic deltas, removal clears, or a forced full replay.
    ///
    /// Send every returned command. Calling this method records submission even if
    /// transport subsequently fails; forced replay repairs best-effort delivery.
    pub fn commands(
        &mut self,
        config: Option<&ZjstatusConfig>,
        snapshot: &ScratchpadStatusSnapshot,
        tab_items: &BTreeMap<usize, Vec<ScratchpadStatusItem>>,
        pane_statuses: &PaneStatusSnapshot,
        force: bool,
    ) -> Vec<String> {
        let mut desired = BTreeMap::new();
        let mut skipped = BTreeSet::new();
        let mut pane_status_globals = BTreeSet::new();
        if let Some(config) = config {
            for (key, output) in &config.outputs {
                if !output.enabled {
                    continue;
                }
                match key {
                    OutputKey::Global(name) => {
                        let key = PublishedKey::Global(name.clone());
                        let value = match output.source {
                            OutputSource::Scratchpad => render(output, snapshot, None),
                            OutputSource::PaneStatus => {
                                pane_status_globals.insert(key.clone());
                                render_pane_status(output, pane_statuses, None)
                            }
                        };
                        if value.is_empty() && !should_publish_empty(output) {
                            // Retire the old status instead of retaining it across a source switch.
                            if self.pane_status_globals.contains(&key) {
                                continue;
                            }
                            // A skipped global empty must not forget the last submission.
                            if let Some(previous) = self.published.get(&key) {
                                skipped.insert(key.clone());
                                desired.insert(key, previous.clone());
                            }
                            continue;
                        }
                        desired.insert(key, value);
                    }
                    OutputKey::Tab(field) => {
                        for (&id, items) in tab_items {
                            desired.insert(
                                PublishedKey::Tab(id, field.clone()),
                                match output.source {
                                    OutputSource::Scratchpad => {
                                        render(output, snapshot, Some(items))
                                    }
                                    OutputSource::PaneStatus => {
                                        render_pane_status(output, pane_statuses, Some(id))
                                    }
                                },
                            );
                        }
                    }
                }
            }
        }

        let mut commands = Vec::new();
        self.retired.retain(|key| {
            let live = match key {
                PublishedKey::Global(_) => true,
                PublishedKey::Tab(id, _) => tab_items.contains_key(id),
            };
            live && !desired.contains_key(key)
        });
        for key in self.published.keys() {
            let clear = match key {
                PublishedKey::Global(_) => self.pane_status_globals.contains(key),
                PublishedKey::Tab(id, _) => tab_items.contains_key(id),
            };
            if clear && !desired.contains_key(key) && self.retired.insert(key.clone()) && !force {
                commands.push(key.command(""));
            }
        }
        if force {
            commands.extend(self.retired.iter().map(|key| key.command("")));
        }
        for (key, value) in &desired {
            // Historical global empties are retained for diffing, never replayed.
            if !skipped.contains(key) && (force || self.published.get(key) != Some(value)) {
                commands.push(key.command(value));
            }
        }
        self.published = desired;
        self.pane_status_globals = pane_status_globals;
        commands
    }

    /// Retired clears need periodic replay even when every output is disabled.
    /// This is the exception to "no active outputs means no replay work". Main
    /// should retain the previous refresh interval (or use 2000 ms if absent) until
    /// these tabs close (global clears persist), then disable replay when no
    /// enabled outputs remain.
    pub fn has_retired(&self) -> bool {
        !self.retired.is_empty()
    }
}

/// Independent work due on a wakeup. Replay flags may both be true; one forced
/// publication satisfies both. Subscriber heartbeat handling remains main's job.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DueTasks {
    /// Poll the external configuration once.
    pub config_poll: bool,
    /// Force publication of desired values and retired clears.
    pub full_replay: bool,
    /// Force publication after discovery of a new receiver.
    pub delayed_replay: bool,
}

#[derive(Debug, Default)]
struct Periodic {
    interval_ms: Option<u64>,
    deadline: Option<Duration>,
}

impl Periodic {
    fn configure(&mut self, now: Duration, interval_ms: Option<u64>) {
        let interval_ms = interval_ms.filter(|&ms| ms != 0);
        if self.interval_ms != interval_ms {
            self.interval_ms = interval_ms;
            self.deadline = interval_ms.and_then(|ms| now.checked_add(Duration::from_millis(ms)));
        }
    }

    fn take_due(&mut self, now: Duration) -> bool {
        if !self.deadline.is_some_and(|deadline| deadline <= now) {
            return false;
        }
        // Coalesce missed periods rather than emitting a catch-up burst.
        self.deadline = self
            .interval_ms
            .and_then(|ms| now.checked_add(Duration::from_millis(ms)));
        true
    }
}

/// Single logical armed wakeup over config, periodic replay and delayed replay.
///
/// Integration sequence: configure/request delays, call `take_due` on every timer
/// wake (including unrelated subscriber wakes), execute its flags, apply any config
/// changes, then call `arm` and pass its returned delay to `set_timeout`. Also call
/// `arm` after configuration or receiver-discovery events. Never schedule a timeout
/// for `None`. Preserve this object across reloads.
///
/// An earlier deadline may supersede an uncancellable timeout. Its later stale wake
/// merely reevaluates deadlines and cannot start another periodic chain. An already
/// armed earlier wake is retained when tasks move later or are disabled.
#[derive(Debug, Default)]
pub struct Scheduler {
    config_poll: Periodic,
    full_replay: Periodic,
    delayed_replay: Option<Duration>,
    armed: Option<Duration>,
}

impl Scheduler {
    /// Set independent millisecond intervals; `None` or zero disables that task.
    /// Unchanged intervals preserve deadlines, so repeated reloads cannot starve
    /// work. A changed interval starts from `now`. Disabling replay also cancels
    /// delayed replay; keep replay enabled when `Publisher::has_retired()` is true.
    pub fn configure(
        &mut self,
        now: Duration,
        config_poll_ms: Option<u64>,
        replay_ms: Option<u64>,
    ) {
        self.config_poll.configure(now, config_poll_ms);
        self.full_replay.configure(now, replay_ms);
        if self.full_replay.interval_ms.is_none() {
            self.delayed_replay = None;
        }
    }

    /// Request a one-shot receiver replay if replay is enabled. Repeated requests
    /// coalesce at the earliest deadline rather than postponing it indefinitely.
    pub fn delay_replay(&mut self, now: Duration, delay: Duration) {
        if self.full_replay.interval_ms.is_some() {
            if let Some(deadline) = now.checked_add(delay) {
                self.delayed_replay = Some(
                    self.delayed_replay
                        .map_or(deadline, |previous| previous.min(deadline)),
                );
            }
        }
    }

    /// Consume only tasks due at this monotonic time. Repeated or early wakes are
    /// harmless; missed periodic intervals produce one flag, not multiple runs.
    pub fn take_due(&mut self, now: Duration) -> DueTasks {
        if self.armed.is_some_and(|deadline| deadline <= now) {
            self.armed = None;
        }
        let delayed_replay = self.delayed_replay.is_some_and(|deadline| deadline <= now);
        if delayed_replay {
            self.delayed_replay = None;
        }
        DueTasks {
            config_poll: self.config_poll.take_due(now),
            full_replay: self.full_replay.take_due(now),
            delayed_replay,
        }
    }

    /// Return the relative delay for a new host timeout only when needed.
    /// Overdue work returns zero; consume it with `take_due` before rearming.
    /// Convert to host seconds with `Duration::as_secs_f64`, not integer division.
    pub fn arm(&mut self, now: Duration) -> Option<Duration> {
        let deadline = [
            self.config_poll.deadline,
            self.full_replay.deadline,
            self.delayed_replay,
        ]
        .into_iter()
        .flatten()
        .min()?;
        if self.armed.is_some_and(|armed| armed <= deadline) {
            return None;
        }
        self.armed = Some(deadline);
        Some(deadline.saturating_sub(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane_status::{PaneStatusItem, PaneStatuses};
    use crate::scratchpad::ScratchpadDisplayState;
    use crate::zjstatus::{parse_zjstatus_config_kdl, ZjstatusConfigLayers};
    use zellij_tile::prelude::{PaneId, TabInfo};

    fn config(input: &str) -> ZjstatusConfig {
        let mut layers = ZjstatusConfigLayers::default();
        layers.push(parse_zjstatus_config_kdl(input).unwrap());
        layers.into_config().unwrap().unwrap()
    }

    fn item(id: usize, state: ScratchpadDisplayState) -> ScratchpadStatusItem {
        ScratchpadStatusItem {
            name: "term".into(),
            title: "term".into(),
            state,
            pane_id: Some(7),
            tab_id: Some(id),
            tab_position: Some(0),
            is_focused: false,
            is_mru: false,
        }
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn receiver(id: u32) -> PaneInfo {
        PaneInfo {
            id,
            is_plugin: true,
            plugin_url: Some("file:/plugins/zjstatus.wasm".into()),
            ..Default::default()
        }
    }

    fn pane_statuses() -> PaneStatusSnapshot {
        PaneStatusSnapshot {
            current_tab_id: Some(42),
            tabs: BTreeMap::from([
                (
                    42,
                    vec![
                        PaneStatusItem {
                            pane_id: PaneId::Terminal(7),
                            title: "build".into(),
                            status: "building".into(),
                            is_focused: true,
                        },
                        PaneStatusItem {
                            pane_id: PaneId::Plugin(8),
                            title: "tests".into(),
                            status: "testing".into(),
                            is_focused: false,
                        },
                    ],
                ),
                (57, vec![]),
            ]),
        }
    }

    #[test]
    fn pane_status_and_scratchpad_outputs_coexist_and_replay_for_new_receivers() {
        let config = config(
            "pipe \"scratch\" { format \"global\"; }\n\
             pipe \"status\" { source \"pane-status\"; }\n\
             tab_pipe \"scratch\" { item_format \"{state}\"; }\n\
             tab_pipe \"status\" { source \"pane-status\"; }",
        );
        let snapshot = ScratchpadStatusSnapshot::default();
        let tabs = BTreeMap::from([
            (42, vec![item(42, ScratchpadDisplayState::Visible)]),
            (57, vec![]),
            (99, vec![]),
        ]);
        let mut statuses = pane_statuses();
        // Status snapshots do not define the live tab set.
        statuses.tabs.insert(123, statuses.tabs[&42].clone());
        let global = render_pane_status(
            &config.outputs[&OutputKey::Global("status".into())],
            &statuses,
            None,
        );
        let tab = render_pane_status(
            &config.outputs[&OutputKey::Tab("status".into())],
            &statuses,
            Some(42),
        );
        assert!(global.contains("building") && global.contains("testing"));
        assert!(tab.contains("building") && tab.contains("testing"));
        let expected = vec![
            "zjstatus::pipe::pipe_scratch::global".into(),
            zjstatus_payload("status", &global),
            "zjstatus::tab_pipe::42::scratch::visible".into(),
            zjstatus_tab_payload(42, "status", &tab),
            "zjstatus::tab_pipe::57::scratch::".into(),
            "zjstatus::tab_pipe::57::status::".into(),
            "zjstatus::tab_pipe::99::scratch::".into(),
            "zjstatus::tab_pipe::99::status::".into(),
        ];
        let mut publisher = Publisher::default();
        assert_eq!(
            publisher.commands(Some(&config), &snapshot, &tabs, &statuses, false),
            expected
        );
        assert!(publisher
            .commands(Some(&config), &snapshot, &tabs, &statuses, false)
            .is_empty());
        let mut receivers = HashSet::new();
        let mut manifest = HashMap::from([(0, vec![receiver(1)])]);
        for position in [0, 1] {
            manifest.insert(position, vec![receiver(position as u32 + 1)]);
            let changed = update_zjstatus_plugin_panes(&mut receivers, &manifest);
            assert!(changed);
            assert_eq!(
                publisher.commands(Some(&config), &snapshot, &tabs, &statuses, changed),
                expected
            );
        }
    }

    #[test]
    fn active_tab_switches_clear_global_status_even_without_active_tab() {
        let config = config("pipe \"status\" { source \"pane-status\"; }");
        let snapshot = ScratchpadStatusSnapshot::default();
        let tabs = BTreeMap::from([(42, vec![]), (57, vec![])]);
        let mut statuses = pane_statuses();
        let mut publisher = Publisher::default();
        let initial = publisher.commands(Some(&config), &snapshot, &tabs, &statuses, false);
        assert_eq!(initial.len(), 1);
        assert!(initial[0].contains("building"));
        for active in [Some(57), None] {
            statuses.current_tab_id = active;
            assert_eq!(
                publisher.commands(Some(&config), &snapshot, &tabs, &statuses, false),
                ["zjstatus::pipe::pipe_status::"]
            );
            assert!(publisher
                .commands(Some(&config), &snapshot, &tabs, &statuses, false)
                .is_empty());
            assert_eq!(
                publisher.commands(Some(&config), &snapshot, &tabs, &statuses, true),
                ["zjstatus::pipe::pipe_status::"]
            );
            statuses.current_tab_id = Some(42);
            assert_eq!(
                publisher.commands(Some(&config), &snapshot, &tabs, &statuses, false),
                initial
            );
        }
    }

    #[test]
    fn moving_and_clearing_pane_status_updates_old_and_new_native_tabs() {
        let config = config("tab_pipe \"status\" { source \"pane-status\"; }");
        let snapshot = ScratchpadStatusSnapshot::default();
        let tabs = BTreeMap::from([(42, vec![]), (57, vec![])]);
        let tab_info = [
            TabInfo {
                tab_id: 42,
                position: 0,
                active: true,
                ..Default::default()
            },
            TabInfo {
                tab_id: 57,
                position: 1,
                ..Default::default()
            },
        ];
        let mut manifest = HashMap::from([(
            0,
            vec![PaneInfo {
                id: 7,
                ..Default::default()
            }],
        )]);
        let mut statuses = PaneStatuses::default();
        statuses
            .set(&["terminal_7", "building"], None, &manifest)
            .unwrap();
        let mut publisher = Publisher::default();
        publisher.commands(
            Some(&config),
            &snapshot,
            &tabs,
            &statuses.snapshot(&manifest, &tab_info),
            false,
        );
        let moved = manifest.remove(&0).unwrap();
        manifest.insert(1, moved);
        statuses.reconcile(&manifest);
        let moved = statuses.snapshot(&manifest, &tab_info);
        let rendered = render_pane_status(
            &config.outputs[&OutputKey::Tab("status".into())],
            &moved,
            Some(57),
        );
        assert!(rendered.contains("building"));
        assert_eq!(
            publisher.commands(Some(&config), &snapshot, &tabs, &moved, false),
            [
                "zjstatus::tab_pipe::42::status::".to_string(),
                zjstatus_tab_payload(57, "status", &rendered),
            ]
        );
        statuses.set(&["terminal_7", ""], None, &manifest).unwrap();
        assert_eq!(
            publisher.commands(
                Some(&config),
                &snapshot,
                &tabs,
                &statuses.snapshot(&manifest, &tab_info),
                false
            ),
            ["zjstatus::tab_pipe::57::status::"]
        );
    }

    #[test]
    fn pane_status_global_removal_disable_and_rename_retire_with_lifetime_replay() {
        let original = config("pipe \"old\" { source \"pane-status\"; }");
        let snapshot = ScratchpadStatusSnapshot::default();
        let statuses = pane_statuses();
        let tabs = BTreeMap::from([(42, vec![])]);
        for replacement in [
            None,
            Some(config(
                "pipe \"old\" { source \"pane-status\"; enabled false; }",
            )),
            Some(config("pipe \"new\" { source \"pane-status\"; }")),
        ] {
            for force in [false, true] {
                let mut publisher = Publisher::default();
                let initial =
                    publisher.commands(Some(&original), &snapshot, &tabs, &statuses, false);
                let removed =
                    publisher.commands(replacement.as_ref(), &snapshot, &tabs, &statuses, force);
                assert_eq!(
                    removed
                        .iter()
                        .filter(|command| *command == "zjstatus::pipe::pipe_old::")
                        .count(),
                    1
                );
                assert!(publisher.has_retired());
                assert!(publisher
                    .commands(replacement.as_ref(), &snapshot, &tabs, &statuses, false)
                    .is_empty());
                assert_eq!(
                    publisher.commands(replacement.as_ref(), &snapshot, &tabs, &statuses, true),
                    removed
                );
                // Removing all configuration and closing every tab cannot prune global clears.
                publisher.commands(
                    None,
                    &snapshot,
                    &BTreeMap::new(),
                    &PaneStatusSnapshot::default(),
                    false,
                );
                let replay = publisher.commands(
                    None,
                    &snapshot,
                    &BTreeMap::new(),
                    &PaneStatusSnapshot::default(),
                    true,
                );
                assert!(replay.contains(&"zjstatus::pipe::pipe_old::".into()));
                assert_eq!(replay.len(), publisher.retired.len());
                let reactivated =
                    publisher.commands(Some(&original), &snapshot, &tabs, &statuses, true);
                assert!(reactivated.contains(&initial[0]));
                assert!(!reactivated.contains(&"zjstatus::pipe::pipe_old::".into()));
            }
        }
    }

    #[test]
    fn source_switch_to_empty_scratchpad_clears_once_and_replays_until_replaced() {
        let status_config = config("pipe \"badge\" { source \"pane-status\"; }");
        let scratch_config = config("pipe \"badge\" { item_format \"{name}\"; }");
        let statuses = pane_statuses();
        let tabs = BTreeMap::from([(42, vec![])]);
        for force in [false, true] {
            let mut snapshot = ScratchpadStatusSnapshot::default();
            let mut publisher = Publisher::default();
            publisher.commands(Some(&status_config), &snapshot, &tabs, &statuses, false);
            assert_eq!(
                publisher.commands(Some(&scratch_config), &snapshot, &tabs, &statuses, force),
                ["zjstatus::pipe::pipe_badge::"]
            );
            assert!(publisher
                .commands(Some(&scratch_config), &snapshot, &tabs, &statuses, false)
                .is_empty());
            assert_eq!(
                publisher.commands(Some(&scratch_config), &snapshot, &tabs, &statuses, true),
                ["zjstatus::pipe::pipe_badge::"]
            );
            snapshot
                .current_items
                .push(item(42, ScratchpadDisplayState::Visible));
            assert_eq!(
                publisher.commands(Some(&scratch_config), &snapshot, &tabs, &statuses, true),
                ["zjstatus::pipe::pipe_badge::term"]
            );
            assert!(!publisher.has_retired());
            assert!(publisher
                .commands(None, &snapshot, &tabs, &statuses, true)
                .is_empty());
        }
    }

    #[test]
    fn source_switch_to_nonempty_scratchpad_never_emits_a_clear() {
        let status_config = config("pipe \"badge\" { source \"pane-status\"; }");
        let scratch_config = config("pipe \"badge\" { item_format \"{name}\"; }");
        let statuses = pane_statuses();
        let tabs = BTreeMap::from([(42, vec![])]);
        let mut snapshot = ScratchpadStatusSnapshot {
            current_items: vec![item(42, ScratchpadDisplayState::Visible)],
            ..Default::default()
        };
        for force in [false, true] {
            let mut publisher = Publisher::default();
            publisher.commands(Some(&status_config), &snapshot, &tabs, &statuses, false);
            assert_eq!(
                publisher.commands(Some(&scratch_config), &snapshot, &tabs, &statuses, force),
                ["zjstatus::pipe::pipe_badge::term"]
            );
            assert!(!publisher.has_retired());
            assert!(publisher
                .commands(None, &snapshot, &tabs, &statuses, true)
                .is_empty());
        }
        // Switching back must restore pane-status empty publication and removal policy.
        let mut publisher = Publisher::default();
        publisher.commands(Some(&scratch_config), &snapshot, &tabs, &statuses, false);
        snapshot.current_items.clear();
        assert_eq!(
            publisher.commands(
                Some(&status_config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false
            ),
            ["zjstatus::pipe::pipe_badge::"]
        );
        assert_eq!(
            publisher.commands(None, &snapshot, &tabs, &statuses, false),
            ["zjstatus::pipe::pipe_badge::"]
        );
        assert!(publisher.has_retired());
    }

    #[test]
    fn receiver_discovery_replays_unchanged_values_and_keeps_delayed_fallback() {
        let config = config(
            "pipe \"badge\" { format \"global\"; }\n\
             tab_pipe \"badge\" { item_format \"{state}\"; }",
        );
        let snapshot = ScratchpadStatusSnapshot::default();
        let tabs = BTreeMap::from([
            (42, vec![item(42, ScratchpadDisplayState::Visible)]),
            (57, vec![item(57, ScratchpadDisplayState::Hidden)]),
        ]);
        let expected = [
            "zjstatus::pipe::pipe_badge::global",
            "zjstatus::tab_pipe::42::badge::visible",
            "zjstatus::tab_pipe::57::badge::hidden",
        ];
        let mut publisher = Publisher::default();
        let mut scheduler = Scheduler::default();
        scheduler.configure(ms(0), None, Some(2000));
        let mut receivers = HashSet::new();
        let mut manifest = HashMap::new();

        for (now, position, id) in [(0, 0, 7), (300, 1, 8)] {
            manifest.insert(position, vec![receiver(id)]);
            let changed = update_zjstatus_plugin_panes(&mut receivers, &manifest);
            assert!(changed);
            assert_eq!(
                publisher.commands(
                    Some(&config),
                    &snapshot,
                    &tabs,
                    &PaneStatusSnapshot::default(),
                    changed
                ),
                expected
            );
            if changed {
                scheduler.delay_replay(ms(now), ms(250));
            }
            assert_eq!(scheduler.arm(ms(now)), Some(ms(250)));

            // Ordinary pane changes do not rediscover the existing receivers.
            manifest.get_mut(&position).unwrap().push(PaneInfo {
                id: 99,
                ..Default::default()
            });
            let changed = update_zjstatus_plugin_panes(&mut receivers, &manifest);
            assert!(!changed);
            assert!(publisher
                .commands(
                    Some(&config),
                    &snapshot,
                    &tabs,
                    &PaneStatusSnapshot::default(),
                    changed
                )
                .is_empty());
            assert_eq!(scheduler.take_due(ms(now + 249)), DueTasks::default());
            assert_eq!(
                scheduler.take_due(ms(now + 250)),
                DueTasks {
                    delayed_replay: true,
                    ..Default::default()
                }
            );
            assert_eq!(scheduler.take_due(ms(now + 250)), DueTasks::default());
        }
        assert!(scheduler.take_due(ms(2000)).full_replay);
    }

    #[test]
    fn receiver_discovery_preserves_empty_retired_and_closed_tab_semantics() {
        let original = config(
            "pipe \"badge\" { item_format \"{name}\"; }\n\
             tab_pipe \"badge\" { item_format \"{state}\"; }\n\
             tab_pipe \"old\" { format \"old\"; }",
        );
        let replacement = config(
            "pipe \"badge\" { item_format \"{name}\"; }\n\
             tab_pipe \"badge\" { item_format \"{state}\"; }",
        );
        let mut snapshot = ScratchpadStatusSnapshot {
            current_items: vec![item(42, ScratchpadDisplayState::Visible)],
            ..Default::default()
        };
        let mut tabs = BTreeMap::from([(42, snapshot.current_items.clone()), (57, vec![])]);
        let mut publisher = Publisher::default();
        let mut receivers = HashSet::new();
        let mut manifest = HashMap::from([(0, vec![receiver(7)])]);
        let changed = update_zjstatus_plugin_panes(&mut receivers, &manifest);
        publisher.commands(
            Some(&original),
            &snapshot,
            &tabs,
            &PaneStatusSnapshot::default(),
            changed,
        );
        publisher.commands(
            Some(&replacement),
            &snapshot,
            &tabs,
            &PaneStatusSnapshot::default(),
            false,
        );
        assert!(publisher.has_retired());

        snapshot.current_items.clear();
        tabs.insert(42, vec![]);
        tabs.remove(&57);
        manifest.insert(1, vec![receiver(8)]);
        let changed = update_zjstatus_plugin_panes(&mut receivers, &manifest);
        assert!(changed);
        assert_eq!(
            publisher.commands(
                Some(&replacement),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                changed
            ),
            [
                "zjstatus::tab_pipe::42::old::",
                "zjstatus::tab_pipe::42::badge::"
            ]
        );
        // Skipping empty global output retains its last submission for diffing.
        snapshot
            .current_items
            .push(item(42, ScratchpadDisplayState::Visible));
        let changed = update_zjstatus_plugin_panes(&mut receivers, &manifest);
        assert!(!changed);
        assert!(publisher
            .commands(
                Some(&replacement),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                changed
            )
            .is_empty());
        assert!(!publisher
            .published
            .keys()
            .chain(&publisher.retired)
            .any(|key| matches!(key, PublishedKey::Tab(57, _))));
    }

    #[test]
    fn first_delta_and_forced_replay_include_each_kind_and_live_tab() {
        let config = config(
            "pipe \"badge\" { format \"global\"; }\n\
             tab_pipe \"badge\" { item_format \"{state}\"; }\n\
             tab_pipe \"count\" { format \"{tab_live_count}\"; }",
        );
        let tabs = BTreeMap::from([
            (42, vec![item(42, ScratchpadDisplayState::Visible)]),
            (57, vec![item(57, ScratchpadDisplayState::Hidden)]),
            (99, vec![]),
        ]);
        let mut publisher = Publisher::default();
        let snapshot = ScratchpadStatusSnapshot::default();
        let expected = vec![
            "zjstatus::pipe::pipe_badge::global",
            "zjstatus::tab_pipe::42::badge::visible",
            "zjstatus::tab_pipe::42::count::1",
            "zjstatus::tab_pipe::57::badge::hidden",
            "zjstatus::tab_pipe::57::count::1",
            "zjstatus::tab_pipe::99::badge::",
            "zjstatus::tab_pipe::99::count::0",
        ];
        assert_eq!(
            publisher.commands(
                Some(&config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false
            ),
            expected
        );
        assert!(publisher
            .commands(
                Some(&config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false
            )
            .is_empty());
        assert_eq!(
            publisher.commands(
                Some(&config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                true
            ),
            expected
        );
    }

    #[test]
    fn empty_transition_and_protocol_encoding() {
        let output_config = config(
            "tab_pipe \"badge\" { item_format \"a::b\\n{state}\\r\"; item_closed_format \"\"; }",
        );
        let mut tabs = BTreeMap::from([(42, vec![item(42, ScratchpadDisplayState::Visible)])]);
        let snapshot = ScratchpadStatusSnapshot::default();
        let mut publisher = Publisher::default();
        assert_eq!(
            publisher.commands(
                Some(&output_config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false
            ),
            ["zjstatus::tab_pipe::42::badge::a::b visible "]
        );
        tabs.get_mut(&42).unwrap()[0].state = ScratchpadDisplayState::Closed;
        assert_eq!(
            publisher.commands(
                Some(&output_config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false
            ),
            ["zjstatus::tab_pipe::42::badge::"]
        );
        assert!(!publisher.has_retired());
        let spaces = config("tab_pipe \"badge\" { format \"  \"; }");
        assert_eq!(
            publisher.commands(
                Some(&spaces),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false
            ),
            ["zjstatus::tab_pipe::42::badge::  "]
        );
    }

    #[test]
    fn removal_disable_rename_retire_and_reactivation() {
        for replacement in [
            None,
            Some(config("tab_pipe \"old\" { enabled false; }")),
            Some(config("tab_pipe \"new\" { format \"new\"; }")),
        ] {
            let original = config("tab_pipe \"old\" { format \"old\"; }");
            let snapshot = ScratchpadStatusSnapshot::default();
            let tabs = BTreeMap::from([(42, vec![])]);
            let mut publisher = Publisher::default();
            publisher.commands(
                Some(&original),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false,
            );
            let removed = publisher.commands(
                replacement.as_ref(),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false,
            );
            assert_eq!(removed[0], "zjstatus::tab_pipe::42::old::");
            assert!(publisher.has_retired());
            assert!(publisher
                .commands(
                    replacement.as_ref(),
                    &snapshot,
                    &tabs,
                    &PaneStatusSnapshot::default(),
                    false
                )
                .is_empty());
            let replay = publisher.commands(
                replacement.as_ref(),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                true,
            );
            assert_eq!(replay[0], "zjstatus::tab_pipe::42::old::");
            let reactivated = publisher.commands(
                Some(&original),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                true,
            );
            assert!(reactivated.contains(&"zjstatus::tab_pipe::42::old::old".into()));
            assert!(!reactivated.contains(&"zjstatus::tab_pipe::42::old::".into()));
            assert!(!publisher
                .retired
                .contains(&PublishedKey::Tab(42, "old".into())));
        }
    }

    #[test]
    fn closed_tabs_prune_active_and_retired_keys_without_clears() {
        let config = config("tab_pipe \"badge\" { format \"x\"; }");
        let snapshot = ScratchpadStatusSnapshot::default();
        for retire in [false, true] {
            let mut publisher = Publisher::default();
            let tabs = BTreeMap::from([(42, vec![])]);
            publisher.commands(
                Some(&config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false,
            );
            if retire {
                publisher.commands(
                    None,
                    &snapshot,
                    &tabs,
                    &PaneStatusSnapshot::default(),
                    false,
                );
            }
            assert!(publisher
                .commands(
                    None,
                    &snapshot,
                    &BTreeMap::new(),
                    &PaneStatusSnapshot::default(),
                    true
                )
                .is_empty());
            assert!(publisher.published.is_empty());
            assert!(!publisher.has_retired());
            assert!(publisher
                .commands(None, &snapshot, &tabs, &PaneStatusSnapshot::default(), true)
                .is_empty());
            assert_eq!(
                publisher
                    .commands(
                        Some(&config),
                        &snapshot,
                        &tabs,
                        &PaneStatusSnapshot::default(),
                        false
                    )
                    .len(),
                1
            );
        }
    }

    #[test]
    fn global_empty_skip_preserves_cache_and_removal_does_not_clear() {
        let config = config("pipe \"badge\" { item_format \"{name}\"; }");
        let mut snapshot = ScratchpadStatusSnapshot::default();
        let tabs = BTreeMap::new();
        let mut publisher = Publisher::default();
        assert!(publisher
            .commands(
                Some(&config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                true
            )
            .is_empty());
        snapshot
            .current_items
            .push(item(42, ScratchpadDisplayState::Visible));
        assert_eq!(
            publisher.commands(
                Some(&config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false
            ),
            ["zjstatus::pipe::pipe_badge::term"]
        );
        snapshot.current_items.clear();
        assert!(publisher
            .commands(
                Some(&config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                true
            )
            .is_empty());
        snapshot
            .current_items
            .push(item(42, ScratchpadDisplayState::Visible));
        assert!(publisher
            .commands(
                Some(&config),
                &snapshot,
                &tabs,
                &PaneStatusSnapshot::default(),
                false
            )
            .is_empty());
        assert_eq!(
            publisher
                .commands(
                    Some(&config),
                    &snapshot,
                    &tabs,
                    &PaneStatusSnapshot::default(),
                    true
                )
                .len(),
            1
        );
        assert!(publisher
            .commands(None, &snapshot, &tabs, &PaneStatusSnapshot::default(), true)
            .is_empty());
        assert!(!publisher.has_retired());
        assert_eq!(
            publisher
                .commands(
                    Some(&config),
                    &snapshot,
                    &tabs,
                    &PaneStatusSnapshot::default(),
                    false
                )
                .len(),
            1
        );
    }

    #[test]
    fn tab_first_or_pane_first_converges_and_positions_do_not_change_keys() {
        let config = config("tab_pipe \"badge\" { item_format \"{state}\"; }");
        let snapshot = ScratchpadStatusSnapshot::default();
        for tab_first in [false, true] {
            let mut publisher = Publisher::default();
            // Main does not call publication at all before its first tab snapshot.
            if tab_first {
                assert_eq!(
                    publisher.commands(
                        Some(&config),
                        &snapshot,
                        &BTreeMap::from([(42, vec![])]),
                        &PaneStatusSnapshot::default(),
                        false
                    ),
                    ["zjstatus::tab_pipe::42::badge::"]
                );
            }
            let mut tabs = BTreeMap::from([(42, vec![item(42, ScratchpadDisplayState::Visible)])]);
            assert_eq!(
                publisher.commands(
                    Some(&config),
                    &snapshot,
                    &tabs,
                    &PaneStatusSnapshot::default(),
                    false
                ),
                ["zjstatus::tab_pipe::42::badge::visible"]
            );
            tabs.get_mut(&42).unwrap()[0].tab_position = Some(5);
            assert!(publisher
                .commands(
                    Some(&config),
                    &snapshot,
                    &tabs,
                    &PaneStatusSnapshot::default(),
                    false
                )
                .is_empty());
            assert!(publisher
                .commands(
                    Some(&config),
                    &snapshot,
                    &BTreeMap::new(),
                    &PaneStatusSnapshot::default(),
                    true
                )
                .is_empty());
        }
    }

    #[test]
    fn independent_periodic_timer_matrix() {
        for poll in [None, Some(100)] {
            for replay in [None, Some(200)] {
                let mut scheduler = Scheduler::default();
                scheduler.configure(ms(0), poll, replay);
                assert_eq!(scheduler.arm(ms(0)), poll.or(replay).map(ms));
                assert_eq!(scheduler.arm(ms(0)), None);
                assert_eq!(scheduler.take_due(ms(99)), DueTasks::default());
                assert_eq!(scheduler.arm(ms(99)), None);
                assert_eq!(scheduler.take_due(ms(100)).config_poll, poll.is_some());
                scheduler.arm(ms(100));
                assert_eq!(
                    scheduler.take_due(ms(200)),
                    DueTasks {
                        config_poll: poll.is_some(),
                        full_replay: replay.is_some(),
                        delayed_replay: false
                    }
                );
                assert_eq!(scheduler.take_due(ms(200)), DueTasks::default());
            }
        }
    }

    #[test]
    fn delayed_replay_supersedes_timer_and_stale_wakes_do_not_multiply_chains() {
        let mut scheduler = Scheduler::default();
        scheduler.configure(ms(0), Some(1000), Some(2000));
        assert_eq!(scheduler.arm(ms(0)), Some(ms(1000)));
        scheduler.delay_replay(ms(0), ms(250));
        assert_eq!(scheduler.arm(ms(0)), Some(ms(250)));
        for now in [10, 20, 100] {
            scheduler.delay_replay(ms(now), ms(250));
            scheduler.configure(ms(now), Some(1000), Some(2000));
            assert_eq!(scheduler.arm(ms(now)), None);
        }
        assert_eq!(
            scheduler.take_due(ms(250)),
            DueTasks {
                delayed_replay: true,
                ..Default::default()
            }
        );
        assert_eq!(scheduler.arm(ms(250)), Some(ms(750)));
        assert!(scheduler.take_due(ms(1000)).config_poll);
        assert_eq!(scheduler.arm(ms(1000)), Some(ms(1000)));
        // Both the original and replacement timeouts may wake at 1000.
        assert_eq!(scheduler.take_due(ms(1000)), DueTasks::default());
        assert_eq!(scheduler.arm(ms(1000)), None);
        scheduler.delay_replay(ms(1750), ms(250));
        assert_eq!(scheduler.arm(ms(1750)), None);
        assert_eq!(
            scheduler.take_due(ms(2000)),
            DueTasks {
                config_poll: true,
                full_replay: true,
                delayed_replay: true
            }
        );
    }

    #[test]
    fn reconfiguration_disable_reenable_and_late_wakes() {
        let mut scheduler = Scheduler::default();
        scheduler.configure(ms(0), Some(100), Some(200));
        assert_eq!(scheduler.arm(ms(0)), Some(ms(100)));
        scheduler.configure(ms(10), Some(300), None);
        scheduler.delay_replay(ms(10), ms(20));
        assert_eq!(scheduler.arm(ms(10)), None);
        assert_eq!(scheduler.take_due(ms(100)), DueTasks::default());
        assert_eq!(scheduler.arm(ms(100)), Some(ms(210)));
        scheduler.configure(ms(110), Some(50), Some(20));
        assert_eq!(scheduler.arm(ms(110)), Some(ms(20)));
        assert!(scheduler.take_due(ms(130)).full_replay);
        assert_eq!(scheduler.arm(ms(130)), Some(ms(20)));
        assert_eq!(
            scheduler.take_due(ms(1000)),
            DueTasks {
                config_poll: true,
                full_replay: true,
                delayed_replay: false
            }
        );
        assert_eq!(scheduler.arm(ms(1000)), Some(ms(20)));
        scheduler.delay_replay(ms(1000), ms(1));
        scheduler.configure(ms(1000), None, None);
        assert_eq!(scheduler.arm(ms(1000)), None);
        assert_eq!(scheduler.take_due(ms(1020)), DueTasks::default());
        assert_eq!(scheduler.arm(ms(1020)), None);
    }

    #[test]
    fn retired_clears_keep_replay_alive_without_active_outputs() {
        let config = config("tab_pipe \"badge\" {}");
        let snapshot = ScratchpadStatusSnapshot::default();
        let tabs = BTreeMap::from([(42, vec![])]);
        let mut publisher = Publisher::default();
        publisher.commands(
            Some(&config),
            &snapshot,
            &tabs,
            &PaneStatusSnapshot::default(),
            false,
        );
        publisher.commands(
            None,
            &snapshot,
            &tabs,
            &PaneStatusSnapshot::default(),
            false,
        );
        let mut scheduler = Scheduler::default();
        scheduler.configure(
            ms(0),
            None,
            publisher
                .has_retired()
                .then_some(u64::from(config.refresh_ms)),
        );
        assert_eq!(scheduler.arm(ms(0)), Some(ms(2000)));
        assert!(scheduler.take_due(ms(2000)).full_replay);
        assert_eq!(
            publisher.commands(None, &snapshot, &tabs, &PaneStatusSnapshot::default(), true),
            ["zjstatus::tab_pipe::42::badge::"]
        );
        publisher.commands(
            None,
            &snapshot,
            &BTreeMap::new(),
            &PaneStatusSnapshot::default(),
            false,
        );
        scheduler.configure(
            ms(2000),
            None,
            publisher
                .has_retired()
                .then_some(u64::from(config.refresh_ms)),
        );
        assert_eq!(scheduler.arm(ms(2000)), None);
    }

    #[test]
    fn zero_intervals_large_intervals_and_overdue_arming() {
        let mut scheduler = Scheduler::default();
        scheduler.configure(ms(0), Some(0), Some(0));
        scheduler.delay_replay(ms(0), Duration::ZERO);
        assert_eq!(scheduler.arm(ms(0)), None);
        scheduler.configure(ms(0), None, Some(u64::from(u32::MAX)));
        assert_eq!(scheduler.arm(ms(0)), Some(ms(u64::from(u32::MAX))));
        scheduler.delay_replay(ms(0), ms(1));
        assert_eq!(scheduler.arm(ms(2)), Some(Duration::ZERO));
        assert!(scheduler.take_due(ms(2)).delayed_replay);
        assert_eq!(scheduler.arm(ms(2)), Some(ms(u64::from(u32::MAX) - 2)));
        scheduler.configure(Duration::MAX, Some(1), Some(1));
        assert_eq!(scheduler.take_due(Duration::MAX), DueTasks::default());
    }
}
