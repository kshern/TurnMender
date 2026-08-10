use crate::core::{
    classify_capacity, CapacityDecision, EventKey, LastAgentMessageState, PolicyState, TaskRegistry,
};
use crate::transport::RETRY_MESSAGE;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CATCH_UP: Duration = Duration::from_secs(10 * 60);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(5);
const MAX_TAIL_BYTES: u64 = 8 * 1024 * 1024;
const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 1024 * 1024;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FailureEvent {
    pub key: EventKey,
    pub completed_at: f64,
    pub error_info: String,
    pub last_agent_message: LastAgentMessageState,
    pub decision: CapacityDecision,
}

#[derive(Debug, Clone)]
enum SessionEvent {
    Started {
        task_id: String,
        turn_id: String,
        started_at: f64,
    },
    UserMessage {
        task_id: String,
        turn_id: String,
        automatic: bool,
        at: f64,
    },
    Completed {
        task_id: String,
        turn_id: String,
        completed_at: f64,
    },
    Failure(FailureEvent),
}

impl SessionEvent {
    fn occurred_at(&self) -> f64 {
        match self {
            Self::Started { started_at, .. } => *started_at,
            Self::UserMessage { at, .. } => *at,
            Self::Completed { completed_at, .. } => *completed_at,
            Self::Failure(event) => event.completed_at,
        }
    }
}

#[derive(Debug)]
pub struct SessionWatcher {
    pub root: PathBuf,
    pub registry: TaskRegistry,
    pub policy: PolicyState,
    offsets: HashMap<PathBuf, u64>,
    partial_lines: HashMap<PathBuf, String>,
    known_files: Vec<PathBuf>,
    last_discovery: Option<Instant>,
    pending: Vec<FailureEvent>,
    pending_keys: HashSet<EventKey>,
    task_names: HashMap<String, String>,
    task_project_paths: HashMap<String, String>,
    policy_dirty: bool,
    initial_cutoff: Option<f64>,
}

impl SessionWatcher {
    pub fn new(root: PathBuf, policy: PolicyState) -> Self {
        Self {
            root,
            registry: TaskRegistry::default(),
            policy,
            offsets: HashMap::new(),
            partial_lines: HashMap::new(),
            known_files: Vec::new(),
            last_discovery: None,
            pending: Vec::new(),
            pending_keys: HashSet::new(),
            task_names: HashMap::new(),
            task_project_paths: HashMap::new(),
            policy_dirty: false,
            initial_cutoff: Some(now() - CATCH_UP.as_secs_f64()),
        }
    }

    pub fn poll(&mut self) -> io::Result<()> {
        if !self.root.exists() {
            fs::create_dir_all(&self.root)?;
        }
        let should_discover = self
            .last_discovery
            .map_or(true, |last| last.elapsed() >= DISCOVERY_INTERVAL);
        if should_discover {
            self.known_files = discover_files(&self.root)?;
            self.refresh_task_names();
            self.refresh_task_project_paths();
            self.last_discovery = Some(Instant::now());
        }

        for path in self.known_files.clone() {
            if !self.offsets.contains_key(&path) {
                let size = fs::metadata(&path)
                    .map(|meta| meta.len())
                    .unwrap_or_default();
                if !is_recent(&path, CATCH_UP) {
                    self.offsets.insert(path, size);
                    continue;
                }
                let start = size.saturating_sub(MAX_TAIL_BYTES);
                self.offsets.insert(path.clone(), start);
            }
            if let Err(error) = self.read_delta(&path) {
                if error.kind() != io::ErrorKind::NotFound {
                    return Err(error);
                }
            }
        }
        self.initial_cutoff = None;
        Ok(())
    }

    pub fn take_candidates(&mut self) -> Vec<FailureEvent> {
        let mut candidates = std::mem::take(&mut self.pending);
        candidates.sort_by(|left, right| {
            left.completed_at
                .partial_cmp(&right.completed_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for event in &candidates {
            self.pending_keys.remove(&event.key);
        }
        candidates
    }

    pub fn requeue(&mut self, event: FailureEvent) {
        if self.pending_keys.insert(event.key.clone()) {
            self.pending.push(event);
        }
    }

    pub fn mark_sent(&mut self, event: &FailureEvent) {
        self.pending_keys.remove(&event.key);
        self.mark_processed(event.key.clone());
    }

    pub fn mark_manual(&mut self, event: &FailureEvent) {
        self.pending_keys.remove(&event.key);
        self.mark_processed(event.key.clone());
    }

    pub fn note_automatic_turn(&mut self, task_id: &str, turn_id: &str) {
        let before = self.policy.chain_failures(task_id);
        self.policy.note_automatic_turn(task_id, turn_id);
        if self.policy.chain_failures(task_id) != before {
            self.policy_dirty = true;
        }
    }

    pub fn policy_is_dirty(&self) -> bool {
        self.policy_dirty
    }

    pub fn mark_policy_saved(&mut self) {
        self.policy_dirty = false;
    }

    fn mark_processed(&mut self, key: EventKey) {
        if self.policy.mark_processed(key) {
            self.policy_dirty = true;
        }
    }

    fn read_delta(&mut self, path: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() {
            return Ok(());
        }
        let size = metadata.len();
        let stored_offset = self.offsets.get(path).copied().unwrap_or_default();
        let offset = if size < stored_offset {
            self.partial_lines.remove(path);
            0
        } else {
            stored_offset
        };
        if size == offset {
            return Ok(());
        }

        let mut file = fs::File::open(path)?;
        file.seek(std::io::SeekFrom::Start(offset))?;
        let mut bytes = Vec::new();
        file.by_ref().take(MAX_READ_BYTES).read_to_end(&mut bytes)?;
        self.offsets
            .insert(path.to_path_buf(), offset + bytes.len() as u64);

        let prefix = self.partial_lines.remove(path).unwrap_or_default();
        let mut text = prefix;
        text.push_str(&String::from_utf8_lossy(&bytes));
        let has_newline = text.ends_with('\n');
        let mut lines: Vec<&str> = text.split('\n').collect();
        if !has_newline {
            if let Some(last) = lines.pop() {
                if last.len() <= MAX_LINE_BYTES {
                    self.partial_lines
                        .insert(path.to_path_buf(), last.to_string());
                }
            }
        } else if lines.last() == Some(&"") {
            lines.pop();
        }

        let Some(task_id) = task_id_from_path(path) else {
            return Ok(());
        };
        for line in lines {
            if line.len() > MAX_LINE_BYTES {
                continue;
            }
            if let Some(event) = parse_line(line, &task_id) {
                self.handle(event);
            }
        }
        Ok(())
    }

    fn handle(&mut self, event: SessionEvent) {
        if self
            .initial_cutoff
            .is_some_and(|cutoff| event.occurred_at() < cutoff)
        {
            return;
        }
        let task_id = match &event {
            SessionEvent::Started { task_id, .. }
            | SessionEvent::UserMessage { task_id, .. }
            | SessionEvent::Completed { task_id, .. } => task_id,
            SessionEvent::Failure(event) => &event.key.task_id,
        };
        if let Some(task_name) = self.task_names.get(task_id) {
            self.registry.set_name(task_id, task_name);
        }
        if let Some(project_path) = self.task_project_paths.get(task_id) {
            self.registry.set_project_path(task_id, project_path);
        }
        match event {
            SessionEvent::Started {
                task_id,
                turn_id,
                started_at,
            } => {
                self.registry.started(&task_id, &turn_id, started_at);
                self.clear_superseded(&task_id, started_at);
            }
            SessionEvent::UserMessage {
                task_id,
                turn_id,
                automatic,
                at,
            } => {
                if automatic {
                    self.note_automatic_turn(&task_id, &turn_id);
                } else {
                    if self.policy.chain_failures(&task_id) > 0 {
                        self.policy.reset_chain(&task_id);
                        self.policy_dirty = true;
                    }
                    let keys: Vec<_> = self
                        .pending
                        .iter()
                        .filter(|failure| failure.key.task_id == task_id)
                        .map(|failure| failure.key.clone())
                        .collect();
                    self.pending
                        .retain(|failure| failure.key.task_id != task_id);
                    for key in keys {
                        self.pending_keys.remove(&key);
                        self.mark_processed(key);
                    }
                    self.registry.started(&task_id, &turn_id, at);
                }
            }
            SessionEvent::Completed {
                task_id,
                turn_id,
                completed_at,
            } => {
                self.clear_superseded(&task_id, completed_at);
                self.registry.completed(&task_id, &turn_id, completed_at);
            }
            SessionEvent::Failure(event) => {
                if self.policy.is_processed(&event.key)
                    || self
                        .registry
                        .latest_started_at(&event.key.task_id)
                        .is_some_and(|at| at > event.completed_at)
                {
                    self.mark_processed(event.key);
                    return;
                }
                match event.decision {
                    CapacityDecision::Eligible => {
                        self.registry.failure_waiting(
                            &event.key.task_id,
                            &event.key.turn_id,
                            event.completed_at,
                        );
                        if self.pending_keys.insert(event.key.clone()) {
                            self.pending.push(event);
                        }
                    }
                    CapacityDecision::CompletedWithOutput => {
                        self.mark_processed(event.key.clone());
                        self.registry.completed_with_output(
                            &event.key.task_id,
                            &event.key.turn_id,
                            event.completed_at,
                        );
                    }
                    CapacityDecision::Unknown => {
                        self.mark_processed(event.key.clone());
                        self.registry.unknown(
                            &event.key.task_id,
                            &event.key.turn_id,
                            event.completed_at,
                        );
                    }
                    CapacityDecision::NotCapacity => {}
                }
            }
        }
    }

    fn clear_superseded(&mut self, task_id: &str, newer_at: f64) {
        let keys: Vec<_> = self
            .pending
            .iter()
            .filter(|failure| failure.key.task_id == task_id && failure.completed_at < newer_at)
            .map(|failure| failure.key.clone())
            .collect();
        if keys.is_empty() {
            return;
        }
        let key_set: HashSet<_> = keys.iter().cloned().collect();
        self.pending
            .retain(|failure| !key_set.contains(&failure.key));
        for key in keys {
            self.pending_keys.remove(&key);
            self.mark_processed(key);
        }
    }

    fn refresh_task_names(&mut self) {
        let Some(parent) = self.root.parent() else {
            return;
        };
        match load_task_names(&parent.join("session_index.jsonl")) {
            Ok(task_names) => {
                self.registry.sync_names(&task_names);
                self.task_names = task_names;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }

    fn refresh_task_project_paths(&mut self) {
        let discovered: Vec<_> = self
            .known_files
            .iter()
            .filter_map(|path| {
                let task_id = task_id_from_path(path)?;
                if self.task_project_paths.contains_key(&task_id) {
                    return None;
                }
                load_task_project_path(path)
                    .ok()
                    .flatten()
                    .map(|project_path| (task_id, project_path))
            })
            .collect();
        for (task_id, project_path) in discovered {
            self.task_project_paths.insert(task_id, project_path);
        }
        self.registry.sync_project_paths(&self.task_project_paths);
    }
}

pub fn default_session_root() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".codex")
        })
        .join("sessions")
}

fn discover_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    visit(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn visit(root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            visit(&path, files)?;
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            files.push(path);
        }
    }
    Ok(())
}

fn is_recent(path: &Path, window: Duration) -> bool {
    let Ok(modified) = fs::metadata(path).and_then(|meta| meta.modified()) else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        <= window
}

fn task_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    let candidate = stem.rsplit('-').take(5).collect::<Vec<_>>();
    if candidate.len() != 5 {
        return None;
    }
    let value = candidate.into_iter().rev().collect::<Vec<_>>().join("-");
    uuid::Uuid::parse_str(&value).ok().map(|_| value)
}

fn load_task_names(path: &Path) -> io::Result<HashMap<String, String>> {
    let file = fs::File::open(path)?;
    let mut task_names = HashMap::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.len() > MAX_LINE_BYTES {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(task_id) = record.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(task_name) = record.get("thread_name").and_then(Value::as_str) else {
            continue;
        };
        let task_name = task_name.trim();
        if !task_name.is_empty() {
            task_names.insert(task_id.to_string(), task_name.to_string());
        }
    }
    Ok(task_names)
}

fn load_task_project_path(path: &Path) -> io::Result<Option<String>> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file).take((MAX_LINE_BYTES + 1) as u64);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.len() > MAX_LINE_BYTES {
        return Ok(None);
    }
    let Ok(record) = serde_json::from_str::<Value>(&line) else {
        return Ok(None);
    };
    if record.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(None);
    }
    let project_path = record
        .get("payload")
        .and_then(|payload| payload.get("cwd"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok(project_path)
}

fn parse_line(line: &str, task_id: &str) -> Option<SessionEvent> {
    let record: Value = serde_json::from_str(line).ok()?;
    let record_type = record.get("type")?.as_str()?;
    let payload = record.get("payload")?;
    let payload_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let timestamp = number(payload.get("started_at"))
        .or_else(|| timestamp(record.get("timestamp")))
        .unwrap_or_else(now);

    if record_type == "event_msg" && payload_type == "task_started" {
        let turn_id = payload.get("turn_id")?.as_str()?.to_string();
        return Some(SessionEvent::Started {
            task_id: task_id.to_string(),
            turn_id,
            started_at: timestamp,
        });
    }

    if record_type == "event_msg" && payload_type == "task_complete" {
        let turn_id = payload.get("turn_id")?.as_str()?.to_string();
        let completed_at = number(payload.get("completed_at")).unwrap_or(timestamp);
        if let Some(error) = payload.get("error").and_then(Value::as_object) {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let error_info = error
                .get("codex_error_info")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let last_agent_message = LastAgentMessageState::from_payload(payload);
            let decision = classify_capacity(message, &error_info, &last_agent_message);
            if decision != CapacityDecision::NotCapacity {
                return Some(SessionEvent::Failure(FailureEvent {
                    key: EventKey {
                        task_id: task_id.to_string(),
                        turn_id,
                    },
                    completed_at,
                    error_info,
                    last_agent_message,
                    decision,
                }));
            }
        }
        return Some(SessionEvent::Completed {
            task_id: task_id.to_string(),
            turn_id,
            completed_at,
        });
    }

    if record_type == "response_item"
        && payload_type == "message"
        && payload.get("role").and_then(Value::as_str) == Some("user")
    {
        let metadata = payload.get("internal_chat_message_metadata_passthrough");
        let turn_id = metadata
            .and_then(|value| value.get("turn_id"))
            .and_then(Value::as_str)
            .or_else(|| payload.get("turn_id").and_then(Value::as_str))?
            .to_string();
        let text = payload
            .get("content")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if is_injected_context_message(&text) {
            return None;
        }
        return Some(SessionEvent::UserMessage {
            task_id: task_id.to_string(),
            turn_id,
            automatic: text.trim() == RETRY_MESSAGE,
            at: timestamp,
        });
    }
    None
}

fn is_injected_context_message(text: &str) -> bool {
    [
        "<recommended_plugins>",
        "# AGENTS.md instructions",
        "<environment_context>",
        "<app-context>",
        "<skills_instructions>",
        "<permissions instructions>",
        "<collaboration_mode>",
        "<apps_instructions>",
        "<plugins_instructions>",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn number(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .or_else(|| value.and_then(Value::as_i64).map(|n| n as f64))
}

fn timestamp(value: Option<&Value>) -> Option<f64> {
    number(value).or_else(|| {
        value
            .and_then(Value::as_str)
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.timestamp_millis() as f64 / 1000.0)
    })
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn temporary_sessions() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("turnmender-watcher-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parses_capacity_with_empty_final_message() {
        let line = serde_json::to_string(&json!({
            "type":"event_msg",
            "timestamp": 10,
            "payload": {
                "type":"task_complete",
                "turn_id":"turn-1",
                "last_agent_message": null,
                "completed_at": 11,
                "error":{"message":"Selected model is at capacity", "codex_error_info":"server_overloaded"}
            }
        })).unwrap();
        let event = parse_line(&line, "task-1").unwrap();
        match event {
            SessionEvent::Failure(failure) => {
                assert_eq!(failure.decision, CapacityDecision::Eligible)
            }
            _ => panic!("wrong event"),
        }
    }

    #[test]
    fn capacity_with_output_is_not_eligible() {
        let line = serde_json::to_string(&json!({
            "type":"event_msg",
            "payload": {
                "type":"task_complete",
                "turn_id":"turn-1",
                "last_agent_message":"已经完成",
                "completed_at": 11,
                "error":{"message":"Selected model is at capacity", "codex_error_info":"server_overloaded"}
            }
        }))
        .unwrap();
        match parse_line(&line, "task-1").unwrap() {
            SessionEvent::Failure(failure) => {
                assert_eq!(failure.decision, CapacityDecision::CompletedWithOutput)
            }
            _ => panic!("wrong event"),
        }
    }

    #[test]
    fn normal_completion_returns_to_idle_event() {
        let line = serde_json::to_string(&json!({
            "type":"event_msg",
            "payload": {
                "type":"task_complete",
                "turn_id":"turn-1",
                "completed_at": 12,
                "last_agent_message":"完成"
            }
        }))
        .unwrap();
        match parse_line(&line, "task-1").unwrap() {
            SessionEvent::Completed {
                turn_id,
                completed_at,
                ..
            } => {
                assert_eq!(turn_id, "turn-1");
                assert_eq!(completed_at, 12.0);
            }
            _ => panic!("wrong event"),
        }
    }

    #[test]
    fn taken_candidate_can_be_requeued() {
        let mut watcher = SessionWatcher::new(PathBuf::new(), PolicyState::default());
        let event = FailureEvent {
            key: EventKey {
                task_id: "task-1".into(),
                turn_id: "turn-1".into(),
            },
            completed_at: 1.0,
            error_info: "server_overloaded".into(),
            last_agent_message: LastAgentMessageState::ExplicitlyEmpty,
            decision: CapacityDecision::Eligible,
        };
        watcher.requeue(event.clone());
        assert_eq!(watcher.take_candidates().len(), 1);
        watcher.requeue(event);
        assert_eq!(watcher.take_candidates().len(), 1);
    }

    #[test]
    fn ignores_injected_context_messages() {
        let line = serde_json::to_string(&json!({
            "type":"response_item",
            "payload": {
                "type":"message",
                "role":"user",
                "internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"},
                "content":[{"type":"input_text", "text":"<environment_context>injected</environment_context>"}]
            }
        }))
        .unwrap();
        assert!(parse_line(&line, "task-1").is_none());
    }

    #[test]
    fn a_user_message_keeps_the_task_running() {
        let mut watcher = SessionWatcher::new(PathBuf::new(), PolicyState::default());
        watcher.initial_cutoff = None;
        watcher.handle(SessionEvent::Started {
            task_id: "task-1".into(),
            turn_id: "turn-1".into(),
            started_at: 10.0,
        });
        watcher.handle(SessionEvent::UserMessage {
            task_id: "task-1".into(),
            turn_id: "turn-1".into(),
            automatic: false,
            at: 11.0,
        });

        let snapshot = watcher.registry.snapshots().pop().unwrap();
        assert_eq!(snapshot.state, crate::core::TaskState::Running);
        assert_eq!(snapshot.latest_turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn parses_record_timestamp_for_user_messages() {
        let parsed = timestamp(Some(&Value::String("2026-08-10T04:42:29.000Z".into()))).unwrap();
        assert_eq!(parsed, 1_786_336_949.0);
    }

    #[test]
    fn waits_for_complete_jsonl_line_before_queueing() {
        let root = temporary_sessions();
        let task_id = uuid::Uuid::new_v4();
        let path = root.join(format!("rollout-2026-08-10T00-00-00-{task_id}.jsonl"));
        let completed_at = now();
        let line = serde_json::to_string(&json!({
            "type":"event_msg",
            "payload": {
                "type":"task_complete",
                "turn_id":"turn-1",
                "last_agent_message":null,
                "completed_at":completed_at,
                "error":{"message":"Selected model is at capacity", "codex_error_info":"server_overloaded"}
            }
        }))
        .unwrap();
        fs::write(&path, &line).unwrap();

        let mut watcher = SessionWatcher::new(root.clone(), PolicyState::default());
        watcher.poll().unwrap();
        assert!(watcher.take_candidates().is_empty());

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file).unwrap();
        watcher.poll().unwrap();
        assert_eq!(watcher.take_candidates().len(), 1);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_does_not_queue_old_failure_from_recent_file() {
        let root = temporary_sessions();
        let task_id = uuid::Uuid::new_v4();
        let path = root.join(format!("rollout-2026-08-10T00-00-00-{task_id}.jsonl"));
        let line = serde_json::to_string(&json!({
            "type":"event_msg",
            "payload": {
                "type":"task_complete",
                "turn_id":"turn-old",
                "last_agent_message":null,
                "completed_at":now() - CATCH_UP.as_secs_f64() - 30.0,
                "error":{"message":"Selected model is at capacity", "codex_error_info":"server_overloaded"}
            }
        }))
        .unwrap();
        fs::write(&path, format!("{line}\n")).unwrap();

        let mut watcher = SessionWatcher::new(root.clone(), PolicyState::default());
        watcher.poll().unwrap();
        assert!(watcher.take_candidates().is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ignores_partial_line_until_next_poll() {
        assert!(parse_line("{", "task-1").is_none());
    }

    #[test]
    fn loads_latest_task_name_from_session_index() {
        let root = temporary_sessions();
        let path = root.join("session_index.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"id\":\"task-1\",\"thread_name\":\"旧名称\"}\n",
                "not json\n",
                "{\"id\":\"task-1\",\"thread_name\":\"重新设计 APP 界面\"}\n"
            ),
        )
        .unwrap();

        let task_names = load_task_names(&path).unwrap();
        assert_eq!(
            task_names.get("task-1").map(String::as_str),
            Some("重新设计 APP 界面")
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loads_project_path_from_session_metadata() {
        let root = temporary_sessions();
        let task_id = uuid::Uuid::new_v4();
        let path = root.join(format!("rollout-2026-08-10T00-00-00-{task_id}.jsonl"));
        let records = [
            json!({
                "type":"session_meta",
                "payload":{"id":task_id, "cwd":"/Users/example/Workspace/TurnMender"}
            }),
            json!({
                "type":"event_msg",
                "timestamp":now(),
                "payload":{"type":"task_started", "turn_id":"turn-1"}
            }),
        ];
        let contents = records
            .into_iter()
            .map(|record| serde_json::to_string(&record).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{contents}\n")).unwrap();

        let mut watcher = SessionWatcher::new(root.clone(), PolicyState::default());
        watcher.poll().unwrap();

        let snapshot = watcher.registry.snapshots().pop().unwrap();
        assert_eq!(
            snapshot.project_path.as_deref(),
            Some("/Users/example/Workspace/TurnMender")
        );

        fs::remove_dir_all(root).unwrap();
    }
}
