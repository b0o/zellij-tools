use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// State that gets persisted across plugin reloads.
/// Uses Vec instead of HashMap because JSON keys must be strings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PersistedState {
    /// (scratchpad_name, tab_id) -> pane_id
    pub panes: Vec<((String, usize), u32)>,
    /// (scratchpad_name, tab_id) -> last focus timestamp
    pub focus_times: Vec<((String, usize), u64)>,
    /// Monotonic counter for focus tracking
    pub focus_counter: u64,
}

/// Get the path to the state file for a given zellij PID.
pub fn state_file_path(zellij_pid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/zellij-tools-{}-state.json", zellij_pid))
}

/// Save state to the state file.
/// Logs error and returns Err if write fails.
pub fn save_state(state: &PersistedState, zellij_pid: u32) -> Result<(), String> {
    let path = state_file_path(zellij_pid);
    let json =
        serde_json::to_string(state).map_err(|e| format!("Failed to serialize state: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write state file {:?}: {}", path, e))
}

/// Load state from the state file.
/// Returns None if file doesn't exist or is corrupted.
pub fn load_state(zellij_pid: u32) -> Option<PersistedState> {
    let path = state_file_path(zellij_pid);
    let contents = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Failed to read state file {:?}: {}", path, e);
            }
            return None;
        }
    };

    match serde_json::from_str(&contents) {
        Ok(state) => Some(state),
        Err(e) => {
            eprintln!("Failed to parse state file {:?}: {}", path, e);
            None
        }
    }
}

/// Delete the state file.
pub fn delete_state_file(zellij_pid: u32) {
    let path = state_file_path(zellij_pid);
    if let Err(e) = fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("Failed to delete state file {:?}: {}", path, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_file_path_uses_pid() {
        let path = state_file_path(12345);
        assert_eq!(path, PathBuf::from("/tmp/zellij-tools-12345-state.json"));
    }

    #[test]
    fn persisted_state_roundtrip() {
        let state = PersistedState {
            panes: vec![(("term".to_string(), 0), 42), (("htop".to_string(), 1), 99)],
            focus_times: vec![(("term".to_string(), 0), 1), (("htop".to_string(), 1), 2)],
            focus_counter: 2,
        };

        let json = serde_json::to_string(&state).unwrap();
        let restored: PersistedState = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.panes.len(), 2);
        assert_eq!(restored.focus_times.len(), 2);
        assert_eq!(restored.focus_counter, 2);
    }

    #[test]
    fn load_nonexistent_returns_none() {
        // Use a PID that's unlikely to have a state file
        let result = load_state(999999999);
        assert!(result.is_none());
    }
}
