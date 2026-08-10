use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Running,
    WaitingContinuation,
    CompletedWithOutput,
    Idle,
    Unknown,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelStatus {
    Ready,
    Unavailable,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task_id: String,
    #[serde(default)]
    pub task_name: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    pub state: TaskState,
    pub latest_turn_id: Option<String>,
    pub last_activity_at: Option<f64>,
    #[serde(default)]
    pub continuation_count: u32,
    pub pending_failure: Option<String>,
    pub channel_status: ChannelStatus,
}

#[derive(Debug, Clone)]
struct TaskRecord {
    snapshot: TaskSnapshot,
    latest_started_at: Option<f64>,
}

#[derive(Debug, Default)]
pub struct TaskRegistry {
    tasks: HashMap<String, TaskRecord>,
}

impl TaskRegistry {
    fn record(&mut self, task_id: &str) -> &mut TaskRecord {
        self.tasks
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRecord {
                snapshot: TaskSnapshot {
                    task_id: task_id.to_string(),
                    task_name: None,
                    project_path: None,
                    state: TaskState::Idle,
                    latest_turn_id: None,
                    last_activity_at: None,
                    continuation_count: 0,
                    pending_failure: None,
                    channel_status: ChannelStatus::Unknown,
                },
                latest_started_at: None,
            })
    }

    pub fn set_name(&mut self, task_id: &str, task_name: &str) {
        let task_name = task_name.trim();
        if !task_name.is_empty() {
            self.record(task_id).snapshot.task_name = Some(task_name.to_string());
        }
    }

    pub fn sync_names(&mut self, task_names: &HashMap<String, String>) {
        for (task_id, record) in &mut self.tasks {
            if let Some(task_name) = task_names.get(task_id) {
                record.snapshot.task_name = Some(task_name.clone());
            }
        }
    }

    pub fn set_project_path(&mut self, task_id: &str, project_path: &str) {
        let project_path = project_path.trim();
        if !project_path.is_empty() {
            self.record(task_id).snapshot.project_path = Some(project_path.to_string());
        }
    }

    pub fn sync_project_paths(&mut self, project_paths: &HashMap<String, String>) {
        for (task_id, record) in &mut self.tasks {
            if let Some(project_path) = project_paths.get(task_id) {
                record.snapshot.project_path = Some(project_path.clone());
            }
        }
    }

    pub fn task_name(&self, task_id: &str) -> Option<&str> {
        self.tasks
            .get(task_id)
            .and_then(|record| record.snapshot.task_name.as_deref())
    }

    pub fn started(&mut self, task_id: &str, turn_id: &str, started_at: f64) {
        let record = self.record(task_id);
        if record
            .latest_started_at
            .map(|value| value > started_at)
            .unwrap_or(false)
        {
            return;
        }
        record.latest_started_at = Some(started_at);
        record.snapshot.latest_turn_id = Some(turn_id.to_string());
        record.snapshot.last_activity_at = Some(started_at);
        record.snapshot.state = TaskState::Running;
        record.snapshot.pending_failure = None;
    }

    pub fn completed(&mut self, task_id: &str, turn_id: &str, completed_at: f64) {
        let record = self.record(task_id);
        if record
            .latest_started_at
            .is_some_and(|started_at| started_at > completed_at)
        {
            return;
        }
        record.snapshot.latest_turn_id = Some(turn_id.to_string());
        record.snapshot.last_activity_at = Some(completed_at);
        record.snapshot.state = TaskState::Idle;
        record.snapshot.pending_failure = None;
    }

    pub fn failure_waiting(&mut self, task_id: &str, turn_id: &str, completed_at: f64) {
        let record = self.record(task_id);
        record.snapshot.latest_turn_id = Some(turn_id.to_string());
        record.snapshot.last_activity_at = Some(completed_at);
        record.snapshot.state = TaskState::WaitingContinuation;
        record.snapshot.pending_failure = Some(turn_id.to_string());
    }

    pub fn completed_with_output(&mut self, task_id: &str, turn_id: &str, completed_at: f64) {
        let record = self.record(task_id);
        record.snapshot.latest_turn_id = Some(turn_id.to_string());
        record.snapshot.last_activity_at = Some(completed_at);
        record.snapshot.state = TaskState::CompletedWithOutput;
        record.snapshot.pending_failure = None;
    }

    pub fn unknown(&mut self, task_id: &str, turn_id: &str, completed_at: f64) {
        let record = self.record(task_id);
        record.snapshot.latest_turn_id = Some(turn_id.to_string());
        record.snapshot.last_activity_at = Some(completed_at);
        record.snapshot.state = TaskState::Unknown;
        record.snapshot.pending_failure = Some(turn_id.to_string());
    }

    pub fn unavailable(&mut self, task_id: &str, turn_id: &str, at: f64) {
        let record = self.record(task_id);
        record.snapshot.latest_turn_id = Some(turn_id.to_string());
        record.snapshot.last_activity_at = Some(at);
        record.snapshot.state = TaskState::Unavailable;
        record.snapshot.pending_failure = Some(turn_id.to_string());
    }

    pub fn sent(&mut self, task_id: &str, turn_id: &str, at: f64) {
        let record = self.record(task_id);
        record.snapshot.latest_turn_id = Some(turn_id.to_string());
        record.snapshot.last_activity_at = Some(at);
        record.snapshot.state = TaskState::Running;
        record.snapshot.pending_failure = None;
    }

    pub fn set_channel_status(&mut self, task_id: &str, status: ChannelStatus) {
        self.record(task_id).snapshot.channel_status = status;
    }

    pub fn set_all_channel_status(&mut self, status: ChannelStatus) {
        for record in self.tasks.values_mut() {
            record.snapshot.channel_status = status;
        }
    }

    pub fn latest_started_at(&self, task_id: &str) -> Option<f64> {
        self.tasks
            .get(task_id)
            .and_then(|record| record.latest_started_at)
    }

    pub fn dismiss(&mut self, task_id: &str) -> bool {
        self.tasks.remove(task_id).is_some()
    }

    pub fn snapshots(&self) -> Vec<TaskSnapshot> {
        let mut values: Vec<_> = self
            .tasks
            .values()
            .map(|record| record.snapshot.clone())
            .collect();
        values.sort_by(|a, b| {
            b.last_activity_at
                .partial_cmp(&a.last_activity_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        values.truncate(100);
        values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_completion_returns_running_task_to_idle() {
        let mut registry = TaskRegistry::default();
        registry.started("task-1", "turn-1", 10.0);
        registry.completed("task-1", "turn-1", 12.0);
        let snapshot = registry.snapshots().pop().unwrap();
        assert_eq!(snapshot.state, TaskState::Idle);
        assert!(snapshot.pending_failure.is_none());
    }

    #[test]
    fn older_completion_does_not_replace_newer_running_turn() {
        let mut registry = TaskRegistry::default();
        registry.started("task-1", "turn-new", 20.0);
        registry.completed("task-1", "turn-old", 12.0);
        let snapshot = registry.snapshots().pop().unwrap();
        assert_eq!(snapshot.state, TaskState::Running);
        assert_eq!(snapshot.latest_turn_id.as_deref(), Some("turn-new"));
    }

    #[test]
    fn task_name_is_included_in_snapshot() {
        let mut registry = TaskRegistry::default();
        registry.started("task-1", "turn-1", 10.0);
        registry.set_name("task-1", "重新设计 APP 界面");

        let snapshot = registry.snapshots().pop().unwrap();
        assert_eq!(snapshot.task_name.as_deref(), Some("重新设计 APP 界面"));
    }

    #[test]
    fn project_path_is_included_in_snapshot() {
        let mut registry = TaskRegistry::default();
        registry.started("task-1", "turn-1", 10.0);
        registry.set_project_path("task-1", "/Users/example/Workspace/CodexGuard");

        let snapshot = registry.snapshots().pop().unwrap();
        assert_eq!(
            snapshot.project_path.as_deref(),
            Some("/Users/example/Workspace/CodexGuard")
        );
    }

    #[test]
    fn dismissed_task_returns_after_new_activity() {
        let mut registry = TaskRegistry::default();
        registry.started("task-1", "turn-1", 10.0);

        assert!(registry.dismiss("task-1"));
        assert!(registry.snapshots().is_empty());
        assert!(!registry.dismiss("missing-task"));

        registry.started("task-1", "turn-2", 20.0);
        let snapshot = registry.snapshots().pop().unwrap();
        assert_eq!(snapshot.latest_turn_id.as_deref(), Some("turn-2"));
    }
}
