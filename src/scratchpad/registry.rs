use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchpadRegistry {
    pub entries: Vec<RegistryRecord>,
    pub focus_counter: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryRecord {
    pub name: String,
    pub tab_id: usize,
    pub state: RegistryRecordState,
    pub updated_at_ms: u64,
    pub owner_plugin_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistryRecordState {
    Present { pane_id: u32 },
    Pending { owner_plugin_id: u32 },
    Tombstone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenDecision {
    Open,
    UseExisting { pane_id: u32 },
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryLockMetadata {
    pub plugin_id: u32,
    pub client_id: u32,
    pub created_ms: u64,
}

#[derive(Debug)]
pub struct RegistryFileLock {
    path: PathBuf,
}

impl Drop for RegistryFileLock {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_file(&self.path) {
            if err.kind() != ErrorKind::NotFound {
                eprintln!("Failed to remove registry lock {:?}: {}", self.path, err);
            }
        }
    }
}

pub fn registry_file_path(zellij_pid: u32) -> PathBuf {
    PathBuf::from(format!("/cache/scratchpad-registry-{}.json", zellij_pid))
}

pub fn registry_lock_path(zellij_pid: u32) -> PathBuf {
    PathBuf::from(format!("/cache/scratchpad-registry-{}.lock", zellij_pid))
}

pub fn registry_temp_file_path(zellij_pid: u32, plugin_id: u32) -> PathBuf {
    PathBuf::from(format!(
        "/cache/scratchpad-registry-{}.json.tmp-{}",
        zellij_pid, plugin_id
    ))
}

impl ScratchpadRegistry {
    pub fn read_from_path(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|err| format!("Failed to parse registry {:?}: {}", path, err)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(format!("Failed to read registry {:?}: {}", path, err)),
        }
    }

    pub fn write_atomic_to_path(&self, path: &Path, temp_path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec(self)
            .map_err(|err| format!("Failed to serialize registry: {}", err))?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(temp_path)
            .map_err(|err| format!("Failed to create temp registry {:?}: {}", temp_path, err))?;
        file.write_all(&json)
            .map_err(|err| format!("Failed to write temp registry {:?}: {}", temp_path, err))?;
        file.sync_all()
            .map_err(|err| format!("Failed to flush temp registry {:?}: {}", temp_path, err))?;
        drop(file);
        fs::rename(temp_path, path).map_err(|err| {
            format!(
                "Failed to replace registry {:?} with {:?}: {}",
                path, temp_path, err
            )
        })
    }

    pub fn begin_open(
        &mut self,
        name: &str,
        tab_id: usize,
        owner_plugin_id: u32,
        now_ms: u64,
        pending_timeout_ms: u64,
    ) -> OpenDecision {
        self.remove_stale_pending(now_ms, pending_timeout_ms);

        if let Some(record) = self.record(name, tab_id) {
            match record.state {
                RegistryRecordState::Present { pane_id } => OpenDecision::UseExisting { pane_id },
                RegistryRecordState::Pending { .. } => OpenDecision::Pending,
                RegistryRecordState::Tombstone => OpenDecision::Open,
            }
        } else {
            self.entries.push(RegistryRecord {
                name: name.to_string(),
                tab_id,
                state: RegistryRecordState::Pending { owner_plugin_id },
                updated_at_ms: now_ms,
                owner_plugin_id,
            });
            OpenDecision::Open
        }
    }

    pub fn finish_open(
        &mut self,
        name: &str,
        tab_id: usize,
        owner_plugin_id: u32,
        pane_id: u32,
        now_ms: u64,
    ) -> bool {
        let Some(record) = self.record_mut(name, tab_id) else {
            return false;
        };
        if record.state != (RegistryRecordState::Pending { owner_plugin_id }) {
            return false;
        }

        record.state = RegistryRecordState::Present { pane_id };
        record.updated_at_ms = now_ms;
        true
    }

    pub fn cancel_open(&mut self, name: &str, tab_id: usize, owner_plugin_id: u32) -> bool {
        let initial_len = self.entries.len();
        self.entries.retain(|record| {
            !(record.name == name
                && record.tab_id == tab_id
                && record.state == (RegistryRecordState::Pending { owner_plugin_id }))
        });
        self.entries.len() != initial_len
    }

    pub fn reconcile(
        &mut self,
        live_tabs: &HashSet<usize>,
        live_panes: &HashMap<u32, usize>,
        now_ms: u64,
        pending_timeout_ms: u64,
    ) {
        self.entries.retain(|record| {
            if !live_tabs.contains(&record.tab_id) {
                return false;
            }

            match record.state {
                RegistryRecordState::Present { pane_id } => {
                    live_panes.get(&pane_id).is_some_and(|tab_id| *tab_id == record.tab_id)
                }
                RegistryRecordState::Pending { .. } => {
                    !is_stale(record.updated_at_ms, now_ms, pending_timeout_ms)
                }
                RegistryRecordState::Tombstone => false,
            }
        });
    }

    pub fn record(&self, name: &str, tab_id: usize) -> Option<&RegistryRecord> {
        self.entries
            .iter()
            .find(|record| record.name == name && record.tab_id == tab_id)
    }

    fn record_mut(&mut self, name: &str, tab_id: usize) -> Option<&mut RegistryRecord> {
        self.entries
            .iter_mut()
            .find(|record| record.name == name && record.tab_id == tab_id)
    }

    fn remove_stale_pending(&mut self, now_ms: u64, pending_timeout_ms: u64) {
        self.entries.retain(|record| match record.state {
            RegistryRecordState::Pending { .. } => {
                !is_stale(record.updated_at_ms, now_ms, pending_timeout_ms)
            }
            _ => true,
        });
    }
}

pub fn acquire_registry_lock(
    path: &Path,
    metadata: &RegistryLockMetadata,
    stale_timeout_ms: u64,
) -> Result<Option<RegistryFileLock>, String> {
    match create_lock_file(path, metadata) {
        Ok(()) => Ok(Some(RegistryFileLock {
            path: path.to_path_buf(),
        })),
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            if lock_is_stale(path, metadata.created_ms, stale_timeout_ms)? {
                fs::remove_file(path)
                    .map_err(|err| format!("Failed to remove stale lock {:?}: {}", path, err))?;
                create_lock_file(path, metadata).map_err(|err| {
                    format!("Failed to create registry lock {:?}: {}", path, err)
                })?;
                Ok(Some(RegistryFileLock {
                    path: path.to_path_buf(),
                }))
            } else {
                Ok(None)
            }
        }
        Err(err) => Err(format!("Failed to create registry lock {:?}: {}", path, err)),
    }
}

fn create_lock_file(path: &Path, metadata: &RegistryLockMetadata) -> Result<(), std::io::Error> {
    let json = serde_json::to_vec(metadata).map_err(std::io::Error::other)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&json)?;
    file.sync_all()
}

fn lock_is_stale(path: &Path, now_ms: u64, stale_timeout_ms: u64) -> Result<bool, String> {
    let contents = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read registry lock {:?}: {}", path, err))?;
    let metadata: RegistryLockMetadata = serde_json::from_str(&contents)
        .map_err(|err| format!("Failed to parse registry lock {:?}: {}", path, err))?;
    Ok(is_stale(metadata.created_ms, now_ms, stale_timeout_ms))
}

fn is_stale(updated_at_ms: u64, now_ms: u64, timeout_ms: u64) -> bool {
    now_ms.saturating_sub(updated_at_ms) >= timeout_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    const OWNER: u32 = 11;
    const TIMEOUT_MS: u64 = 2_000;

    fn live_tabs(tabs: &[usize]) -> HashSet<usize> {
        tabs.iter().copied().collect()
    }

    fn live_panes(panes: &[(u32, usize)]) -> HashMap<u32, usize> {
        panes.iter().copied().collect()
    }

    fn temp_file(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "zellij-tools-registry-{}-{}-{}",
            std::process::id(),
            now,
            name
        ))
    }

    #[test]
    fn missing_key_creates_pending() {
        let mut registry = ScratchpadRegistry::default();

        let decision = registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);

        assert_eq!(decision, OpenDecision::Open);
        assert_eq!(
            registry.record("term", 7).map(|record| &record.state),
            Some(&RegistryRecordState::Pending {
                owner_plugin_id: OWNER
            })
        );
    }

    #[test]
    fn own_pending_becomes_present() {
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);

        assert!(registry.finish_open("term", 7, OWNER, 42, 150));

        assert_eq!(
            registry.record("term", 7).map(|record| &record.state),
            Some(&RegistryRecordState::Present { pane_id: 42 })
        );
    }

    #[test]
    fn stale_pending_is_removed() {
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);

        registry.reconcile(&live_tabs(&[7]), &live_panes(&[]), 2_100, TIMEOUT_MS);

        assert!(registry.record("term", 7).is_none());
    }

    #[test]
    fn fresh_pending_prevents_duplicate_open() {
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);

        let decision = registry.begin_open("term", 7, 99, 150, TIMEOUT_MS);

        assert_eq!(decision, OpenDecision::Pending);
        assert_eq!(registry.entries.len(), 1);
    }

    #[test]
    fn present_resolves_to_existing_pane() {
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);
        registry.finish_open("term", 7, OWNER, 42, 150);

        let decision = registry.begin_open("term", 7, 99, 200, TIMEOUT_MS);

        assert_eq!(decision, OpenDecision::UseExisting { pane_id: 42 });
    }

    #[test]
    fn missing_pane_is_reconciled_out() {
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);
        registry.finish_open("term", 7, OWNER, 42, 150);

        registry.reconcile(&live_tabs(&[7]), &live_panes(&[]), 200, TIMEOUT_MS);

        assert!(registry.record("term", 7).is_none());
    }

    #[test]
    fn missing_tab_is_reconciled_out() {
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);
        registry.finish_open("term", 7, OWNER, 42, 150);

        registry.reconcile(&live_tabs(&[8]), &live_panes(&[(42, 7)]), 200, TIMEOUT_MS);

        assert!(registry.record("term", 7).is_none());
    }

    #[test]
    fn tab_local_identity_keeps_same_name_separate() {
        let mut registry = ScratchpadRegistry::default();

        let first = registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);
        let second = registry.begin_open("term", 8, OWNER, 100, TIMEOUT_MS);

        assert_eq!(first, OpenDecision::Open);
        assert_eq!(second, OpenDecision::Open);
        assert!(registry.record("term", 7).is_some());
        assert!(registry.record("term", 8).is_some());
    }

    #[test]
    fn lock_acquisition_succeeds_once() {
        let path = temp_file("lock-once");
        let metadata = RegistryLockMetadata {
            plugin_id: OWNER,
            client_id: 1,
            created_ms: 100,
        };

        let lock = acquire_registry_lock(&path, &metadata, TIMEOUT_MS).unwrap();

        assert!(lock.is_some());
        assert!(path.exists());
    }

    #[test]
    fn second_lock_acquisition_fails_while_lock_exists() {
        let path = temp_file("lock-second");
        let metadata = RegistryLockMetadata {
            plugin_id: OWNER,
            client_id: 1,
            created_ms: 100,
        };
        let _lock = acquire_registry_lock(&path, &metadata, TIMEOUT_MS).unwrap();

        let second = acquire_registry_lock(
            &path,
            &RegistryLockMetadata {
                created_ms: 150,
                ..metadata
            },
            TIMEOUT_MS,
        )
        .unwrap();

        assert!(second.is_none());
    }

    #[test]
    fn stale_lock_is_removed() {
        let path = temp_file("lock-stale");
        let old_metadata = RegistryLockMetadata {
            plugin_id: OWNER,
            client_id: 1,
            created_ms: 100,
        };
        drop(acquire_registry_lock(&path, &old_metadata, TIMEOUT_MS).unwrap().unwrap());
        fs::write(&path, serde_json::to_vec(&old_metadata).unwrap()).unwrap();

        let new_metadata = RegistryLockMetadata {
            plugin_id: 99,
            client_id: 2,
            created_ms: 2_100,
        };
        let lock = acquire_registry_lock(&path, &new_metadata, TIMEOUT_MS).unwrap();

        assert!(lock.is_some());
        let restored: RegistryLockMetadata = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(restored, new_metadata);
    }

    #[test]
    fn registry_write_read_round_trips() {
        let path = temp_file("registry-json");
        let temp_path = temp_file("registry-json-temp");
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);
        registry.finish_open("term", 7, OWNER, 42, 150);

        registry.write_atomic_to_path(&path, &temp_path).unwrap();
        let restored = ScratchpadRegistry::read_from_path(&path).unwrap();

        assert_eq!(restored, registry);
    }

    #[test]
    fn atomic_write_leaves_valid_registry_content() {
        let path = temp_file("registry-atomic");
        let temp_path = temp_file("registry-atomic-temp");
        let mut registry = ScratchpadRegistry::default();
        registry.begin_open("term", 7, OWNER, 100, TIMEOUT_MS);
        registry.write_atomic_to_path(&path, &temp_path).unwrap();
        registry.finish_open("term", 7, OWNER, 42, 150);

        registry.write_atomic_to_path(&path, &temp_path).unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        let restored: ScratchpadRegistry = serde_json::from_str(&contents).unwrap();

        assert_eq!(restored.record("term", 7).map(|record| &record.state), Some(&RegistryRecordState::Present { pane_id: 42 }));
    }
}
