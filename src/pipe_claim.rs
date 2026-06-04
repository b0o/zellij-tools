use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use zellij_tile::prelude::PipeMessage;

pub fn broadcast_pipe_claim_key(pipe_message: &PipeMessage) -> String {
    let mut hash = DefaultHasher::new();
    pipe_message.source.hash(&mut hash);
    pipe_message.name.hash(&mut hash);
    pipe_message.payload.hash(&mut hash);
    pipe_message.args.hash(&mut hash);
    pipe_message.is_private.hash(&mut hash);
    format!("broadcast-{:016x}", hash.finish())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use zellij_tile::prelude::{PipeMessage, PipeSource};

    use super::*;

    fn keybind_message(name: &str, payload: &str) -> PipeMessage {
        PipeMessage {
            source: PipeSource::Keybind,
            name: name.to_string(),
            payload: Some(payload.to_string()),
            args: BTreeMap::new(),
            is_private: true,
        }
    }

    #[test]
    fn broadcast_pipe_claim_key_is_stable_for_identical_messages() {
        let message = keybind_message(
            "scratchpad-toggle",
            "zellij-tools::scratchpad::toggle::term",
        );

        assert_eq!(
            broadcast_pipe_claim_key(&message),
            broadcast_pipe_claim_key(&message)
        );
    }

    #[test]
    fn broadcast_pipe_claim_key_changes_for_different_messages() {
        let term = keybind_message(
            "scratchpad-toggle",
            "zellij-tools::scratchpad::toggle::term",
        );
        let logs = keybind_message(
            "scratchpad-toggle",
            "zellij-tools::scratchpad::toggle::logs",
        );

        assert_ne!(
            broadcast_pipe_claim_key(&term),
            broadcast_pipe_claim_key(&logs)
        );
    }
}
