use crate::core::{ChannelStatus, TaskSnapshot};
use crate::storage::{
    append_log, load_config, load_policy, save_config, save_policy, GuardConfig, Paths,
};
use crate::transport::{make_sender, SendRequest, Sender, RETRY_MESSAGE};
use crate::watcher::{default_session_root, FailureEvent, SessionWatcher};
use parking_lot::Mutex;
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_AUTOMATIC_CHAIN: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardStatusKind {
    Preparing,
    WatchFailed,
    Watching,
    WatchingUnsupported,
    WatchingChannelUnavailable,
    TaskWaiting,
    ChainProtected,
    ManualContinue,
    Continuing,
    Continued,
    ConfirmSend,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardStatus {
    pub kind: GuardStatusKind,
    pub task_name: Option<String>,
    pub detail: Option<String>,
}

impl GuardStatus {
    fn new(kind: GuardStatusKind) -> Self {
        Self {
            kind,
            task_name: None,
            detail: None,
        }
    }

    fn for_task(kind: GuardStatusKind, task_name: Option<&str>) -> Self {
        Self {
            kind,
            task_name: compact_task_name(task_name),
            detail: None,
        }
    }

    fn watch_failed(error: impl ToString) -> Self {
        Self {
            kind: GuardStatusKind::WatchFailed,
            task_name: None,
            detail: Some(error.to_string()),
        }
    }

    fn watching(channel_status: ChannelStatus) -> Self {
        match channel_status {
            ChannelStatus::Ready => Self::new(GuardStatusKind::Watching),
            ChannelStatus::Unsupported => Self::new(GuardStatusKind::WatchingUnsupported),
            _ => Self::new(GuardStatusKind::WatchingChannelUnavailable),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardSnapshot {
    pub running: bool,
    pub auto_retry_enabled: bool,
    pub platform: String,
    pub session_root: String,
    pub log_path: String,
    pub channel_status: ChannelStatus,
    pub status: GuardStatus,
    pub tasks: Vec<TaskSnapshot>,
}

struct Runtime {
    running: bool,
    auto_retry_enabled: bool,
    status: GuardStatus,
    paths: Paths,
    watcher: SessionWatcher,
    sender: Arc<dyn Sender>,
    last_watch_error: Option<String>,
    last_policy_error: Option<String>,
}

impl Runtime {
    fn new() -> Self {
        let paths = Paths::discover(default_session_root());
        let policy = load_policy(&paths.state);
        let config = load_config(&paths.config);
        Self {
            running: false,
            auto_retry_enabled: config.auto_retry_enabled,
            status: GuardStatus::new(GuardStatusKind::Preparing),
            watcher: SessionWatcher::new(paths.session_root.clone(), policy),
            sender: make_sender(),
            paths,
            last_watch_error: None,
            last_policy_error: None,
        }
    }

    fn snapshot(&self) -> GuardSnapshot {
        let mut tasks = self.watcher.registry.snapshots();
        for task in &mut tasks {
            task.continuation_count = self.watcher.policy.chain_failures(&task.task_id);
        }
        GuardSnapshot {
            running: self.running,
            auto_retry_enabled: self.auto_retry_enabled,
            platform: std::env::consts::OS.to_string(),
            session_root: self.paths.session_root.display().to_string(),
            log_path: self.paths.log.display().to_string(),
            channel_status: self.sender.status(),
            status: self.status.clone(),
            tasks,
        }
    }

    fn poll(&mut self) {
        if let Err(error) = self.watcher.poll() {
            let error = error.to_string();
            self.status = GuardStatus::watch_failed(&error);
            if self.last_watch_error.as_deref() != Some(error.as_str()) {
                self.log(&format!("任务记录监听失败：{error}"));
            }
            self.last_watch_error = Some(error);
            return;
        }
        let channel_status = self.sender.status();
        if self.last_watch_error.take().is_some() {
            self.status = GuardStatus::watching(channel_status);
            self.log("任务记录监听已恢复");
        }
        self.watcher.registry.set_all_channel_status(channel_status);
        let candidates = self.watcher.take_candidates();
        if candidates.is_empty() {
            if self.running && self.status.kind == GuardStatusKind::Preparing {
                self.status = GuardStatus::watching(channel_status);
            }
            self.persist_policy();
            return;
        }
        for event in candidates {
            self.process_candidate(event, channel_status);
            self.persist_policy();
        }
    }

    fn process_candidate(&mut self, event: FailureEvent, channel_status: ChannelStatus) {
        let task_name = compact_task_name(self.watcher.registry.task_name(&event.key.task_id));
        if !self.auto_retry_enabled {
            self.watcher.requeue(event);
            self.status = GuardStatus::for_task(GuardStatusKind::TaskWaiting, task_name.as_deref());
            return;
        }
        let chain = self.watcher.policy.chain_failures(&event.key.task_id);
        if chain >= MAX_AUTOMATIC_CHAIN {
            self.watcher.mark_manual(&event);
            self.watcher.registry.unknown(
                &event.key.task_id,
                &event.key.turn_id,
                event.completed_at,
            );
            self.status =
                GuardStatus::for_task(GuardStatusKind::ChainProtected, task_name.as_deref());
            self.log(&format!(
                "任务 {} 连续自动继续达到上限，转为人工处理",
                event.key.task_id
            ));
            return;
        }
        if channel_status != ChannelStatus::Ready {
            self.watcher.mark_manual(&event);
            self.watcher.registry.unavailable(
                &event.key.task_id,
                &event.key.turn_id,
                event.completed_at,
            );
            self.status =
                GuardStatus::for_task(GuardStatusKind::ManualContinue, task_name.as_deref());
            self.log(&format!(
                "任务 {} 消息通道不可用，未切换到其他发送方式",
                event.key.task_id
            ));
            return;
        }

        self.status = GuardStatus::for_task(GuardStatusKind::Continuing, task_name.as_deref());
        let request = SendRequest {
            task_id: event.key.task_id.clone(),
            failed_turn_id: event.key.turn_id.clone(),
            message: RETRY_MESSAGE.to_string(),
        };
        match self.sender.send(&request) {
            Ok(receipt) if receipt.accepted && !receipt.new_turn_id.is_empty() => {
                self.watcher.mark_sent(&event);
                self.watcher
                    .note_automatic_turn(&event.key.task_id, &receipt.new_turn_id);
                self.watcher
                    .registry
                    .sent(&event.key.task_id, &receipt.new_turn_id, now());
                self.status =
                    GuardStatus::for_task(GuardStatusKind::Continued, task_name.as_deref());
                self.log(&format!(
                    "任务 {} 自动继续成功：失败轮次 {}，新轮次 {}",
                    event.key.task_id, event.key.turn_id, receipt.new_turn_id
                ));
            }
            Ok(_) => {
                self.watcher.mark_manual(&event);
                self.watcher.registry.unknown(
                    &event.key.task_id,
                    &event.key.turn_id,
                    event.completed_at,
                );
                self.status =
                    GuardStatus::for_task(GuardStatusKind::ConfirmSend, task_name.as_deref());
                self.log(&format!(
                    "任务 {} 回执不完整，停止自动补发",
                    event.key.task_id
                ));
            }
            Err(error) => {
                self.watcher.mark_manual(&event);
                self.watcher.registry.unavailable(
                    &event.key.task_id,
                    &event.key.turn_id,
                    event.completed_at,
                );
                self.status =
                    GuardStatus::for_task(GuardStatusKind::ManualContinue, task_name.as_deref());
                self.log(&format!("任务 {} 发送失败：{error}", event.key.task_id));
            }
        }
    }

    fn log(&self, message: &str) {
        let _ = append_log(&self.paths.log, message);
    }

    fn persist_policy(&mut self) {
        if !self.watcher.policy_is_dirty() {
            return;
        }
        match save_policy(&self.paths.state, &mut self.watcher.policy) {
            Ok(()) => {
                self.watcher.mark_policy_saved();
                if self.last_policy_error.take().is_some() {
                    self.log("守护状态保存已恢复");
                }
            }
            Err(error) => {
                let error = error.to_string();
                if self.last_policy_error.as_deref() != Some(error.as_str()) {
                    self.log(&format!("无法保存守护状态：{error}"));
                }
                self.last_policy_error = Some(error);
            }
        }
    }
}

pub struct GuardService {
    runtime: Arc<Mutex<Runtime>>,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl GuardService {
    pub fn new() -> Self {
        Self {
            runtime: Arc::new(Mutex::new(Runtime::new())),
            stop: Arc::new(AtomicBool::new(false)),
            worker: Mutex::new(None),
        }
    }

    pub fn start(&self) {
        let mut worker = self.worker.lock();
        if worker.is_some() {
            return;
        }
        self.stop.store(false, Ordering::Relaxed);
        {
            let mut runtime = self.runtime.lock();
            runtime.running = true;
            runtime.status = GuardStatus::new(GuardStatusKind::Preparing);
        }
        let runtime = Arc::clone(&self.runtime);
        let stop = Arc::clone(&self.stop);
        *worker = Some(thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Some(mut runtime) = runtime.try_lock() {
                    if runtime.running {
                        runtime.poll();
                    }
                }
                thread::sleep(Duration::from_secs(2));
            }
        }));
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        {
            let mut runtime = self.runtime.lock();
            runtime.running = false;
            runtime.status = GuardStatus::new(GuardStatusKind::Stopped);
        }
        if let Some(handle) = self.worker.lock().take() {
            let _ = handle.join();
        }
    }

    pub fn set_auto_retry(&self, enabled: bool) {
        let mut runtime = self.runtime.lock();
        runtime.auto_retry_enabled = enabled;
        if let Err(error) = save_config(
            &runtime.paths.config,
            &GuardConfig {
                auto_retry_enabled: enabled,
            },
        ) {
            runtime.log(&format!("无法保存自动继续设置：{error}"));
        }
    }

    pub fn dismiss_task(&self, task_id: &str) -> bool {
        let task_id = task_id.trim();
        if task_id.is_empty() {
            return false;
        }
        let mut runtime = self.runtime.lock();
        runtime.watcher.registry.dismiss(task_id)
    }

    pub fn snapshot(&self) -> GuardSnapshot {
        self.runtime.lock().snapshot()
    }
}

impl Default for GuardService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GuardService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn compact_task_name(task_name: Option<&str>) -> Option<String> {
    let task_name = task_name?.trim();
    if task_name.is_empty() {
        return None;
    }
    let mut characters = task_name.chars();
    let mut compact: String = characters.by_ref().take(24).collect();
    if characters.next().is_some() {
        compact.push('…');
    }
    Some(compact)
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
