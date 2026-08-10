use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventKey {
    pub task_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PolicyState {
    pub processed: HashSet<EventKey>,
    pub automatic_turns: HashSet<String>,
    pub automatic_chain_failures: HashMap<String, u32>,
}

impl PolicyState {
    pub fn is_processed(&self, key: &EventKey) -> bool {
        self.processed.contains(key)
    }

    pub fn mark_processed(&mut self, key: EventKey) -> bool {
        self.processed.insert(key)
    }

    pub fn note_automatic_turn(&mut self, task_id: &str, turn_id: &str) {
        if self.automatic_turns.insert(turn_id.to_string()) {
            *self
                .automatic_chain_failures
                .entry(task_id.to_string())
                .or_default() += 1;
        }
    }

    pub fn is_automatic_turn(&self, turn_id: &str) -> bool {
        self.automatic_turns.contains(turn_id)
    }

    pub fn chain_failures(&self, task_id: &str) -> u32 {
        self.automatic_chain_failures
            .get(task_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn reset_chain(&mut self, task_id: &str) {
        self.automatic_chain_failures.remove(task_id);
    }

    pub fn trim(&mut self, max_processed: usize) {
        if self.processed.len() > max_processed {
            let mut keys: Vec<_> = self.processed.drain().collect();
            keys.sort_by(|a, b| a.task_id.cmp(&b.task_id).then(a.turn_id.cmp(&b.turn_id)));
            self.processed
                .extend(keys.into_iter().rev().take(max_processed));
        }
        const MAX_AUTOMATIC_TURNS: usize = 10_000;
        if self.automatic_turns.len() > MAX_AUTOMATIC_TURNS {
            let mut turns: Vec<_> = self.automatic_turns.drain().collect();
            turns.sort();
            self.automatic_turns
                .extend(turns.into_iter().rev().take(MAX_AUTOMATIC_TURNS));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(task: &str, turn: &str) -> EventKey {
        EventKey {
            task_id: task.into(),
            turn_id: turn.into(),
        }
    }

    #[test]
    fn deduplicates_events_and_tracks_automatic_chain() {
        let mut state = PolicyState::default();
        assert!(state.mark_processed(key("a", "1")));
        assert!(!state.mark_processed(key("a", "1")));
        state.note_automatic_turn("a", "2");
        state.note_automatic_turn("a", "2");
        assert_eq!(state.chain_failures("a"), 1);
        assert!(state.is_automatic_turn("2"));
        state.reset_chain("a");
        assert_eq!(state.chain_failures("a"), 0);
    }
}
