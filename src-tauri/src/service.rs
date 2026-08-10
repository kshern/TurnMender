use crate::core::{ChannelStatus, TaskSnapshot};
use crate::storage::{
    append_log, load_config, load_policy, save_config, save_policy, ContinuationConfig, Paths,
    MAX_AUTOMATIC_CHAIN_LIMIT, MIN_AUTOMATIC_CHAIN_LIMIT,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationStatusKind {
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
pub struct ContinuationStatus {
    pub kind: ContinuationStatusKind,
    pub detail: Option<String>,
}

impl ContinuationStatus {
    fn new(kind: ContinuationStatusKind) -> Self {
        Self { kind, detail: None }
    }

    fn watch_failed(error: impl ToString) -> Self {
        Self {
            kind: ContinuationStatusKind::WatchFailed,
            detail: Some(error.to_string()),
        }
    }

    fn watching(channel_status: ChannelStatus) -> Self {
        match channel_status {
            ChannelStatus::Ready => Self::new(ContinuationStatusKind::Watching),
            ChannelStatus::Unsupported => Self::new(ContinuationStatusKind::WatchingUnsupported),
            _ => Self::new(ContinuationStatusKind::WatchingChannelUnavailable),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuationSnapshot {
    pub running: bool,
    pub auto_retry_enabled: bool,
    pub automatic_chain_limit: u32,
    pub automatic_chain_limit_min: u32,
    pub automatic_chain_limit_max: u32,
    pub platform: String,
    pub session_root: String,
    pub log_path: String,
    pub channel_status: ChannelStatus,
    pub status: ContinuationStatus,
    pub tasks: Vec<TaskSnapshot>,
}

struct Runtime {
    running: bool,
    auto_retry_enabled: bool,
    automatic_chain_limit: u32,
    status: ContinuationStatus,
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
            automatic_chain_limit: config.automatic_chain_limit,
            status: ContinuationStatus::new(ContinuationStatusKind::Preparing),
            watcher: SessionWatcher::new(paths.session_root.clone(), policy),
            sender: make_sender(),
            paths,
            last_watch_error: None,
            last_policy_error: None,
        }
    }

    fn snapshot(&self) -> ContinuationSnapshot {
        let mut tasks = self.watcher.registry.snapshots();
        for task in &mut tasks {
            task.continuation_count = self.watcher.policy.chain_failures(&task.task_id);
        }
        ContinuationSnapshot {
            running: self.running,
            auto_retry_enabled: self.auto_retry_enabled,
            automatic_chain_limit: self.automatic_chain_limit,
            automatic_chain_limit_min: MIN_AUTOMATIC_CHAIN_LIMIT,
            automatic_chain_limit_max: MAX_AUTOMATIC_CHAIN_LIMIT,
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
            self.status = ContinuationStatus::watch_failed(&error);
            if self.last_watch_error.as_deref() != Some(error.as_str()) {
                self.log(&format!("任务记录监听失败：{error}"));
            }
            self.last_watch_error = Some(error);
            return;
        }
        let channel_status = self.sender.status();
        if self.last_watch_error.take().is_some() {
            self.status = ContinuationStatus::watching(channel_status);
            self.log("任务记录监听已恢复");
        }
        self.watcher.registry.set_all_channel_status(channel_status);
        let candidates = self.watcher.take_candidates();
        if candidates.is_empty() {
            if self.running && self.status.kind == ContinuationStatusKind::Preparing {
                self.status = ContinuationStatus::watching(channel_status);
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
        if !self.auto_retry_enabled {
            self.watcher.requeue(event);
            self.status = ContinuationStatus::new(ContinuationStatusKind::TaskWaiting);
            return;
        }
        let chain = self.watcher.policy.chain_failures(&event.key.task_id);
        if chain >= self.automatic_chain_limit {
            self.watcher.mark_manual(&event);
            self.watcher.registry.unknown(
                &event.key.task_id,
                &event.key.turn_id,
                event.completed_at,
            );
            self.status = ContinuationStatus::new(ContinuationStatusKind::ChainProtected);
            self.log(&format!(
                "任务 {} 连续自动继续达到上限（{} 次），转为人工处理",
                event.key.task_id, self.automatic_chain_limit
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
            self.status = ContinuationStatus::new(ContinuationStatusKind::ManualContinue);
            self.log(&format!(
                "任务 {} 消息通道不可用，未切换到其他发送方式",
                event.key.task_id
            ));
            return;
        }

        self.status = ContinuationStatus::new(ContinuationStatusKind::Continuing);
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
                self.status = ContinuationStatus::new(ContinuationStatusKind::Continued);
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
                self.status = ContinuationStatus::new(ContinuationStatusKind::ConfirmSend);
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
                self.status = ContinuationStatus::new(ContinuationStatusKind::ManualContinue);
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
                    self.log("续行状态保存已恢复");
                }
            }
            Err(error) => {
                let error = error.to_string();
                if self.last_policy_error.as_deref() != Some(error.as_str()) {
                    self.log(&format!("无法保存续行状态：{error}"));
                }
                self.last_policy_error = Some(error);
            }
        }
    }
}

pub struct ContinuationService {
    runtime: Arc<Mutex<Runtime>>,
    stop: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ContinuationService {
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
            runtime.status = ContinuationStatus::new(ContinuationStatusKind::Preparing);
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
            runtime.status = ContinuationStatus::new(ContinuationStatusKind::Stopped);
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
            &ContinuationConfig {
                auto_retry_enabled: enabled,
                automatic_chain_limit: runtime.automatic_chain_limit,
            },
        ) {
            runtime.log(&format!("无法保存自动继续设置：{error}"));
        }
    }

    pub fn set_automatic_chain_limit(&self, limit: u32) -> u32 {
        let mut runtime = self.runtime.lock();
        let limit = limit.clamp(MIN_AUTOMATIC_CHAIN_LIMIT, MAX_AUTOMATIC_CHAIN_LIMIT);
        runtime.automatic_chain_limit = limit;
        if let Err(error) = save_config(
            &runtime.paths.config,
            &ContinuationConfig {
                auto_retry_enabled: runtime.auto_retry_enabled,
                automatic_chain_limit: limit,
            },
        ) {
            runtime.log(&format!("无法保存连续自动继续上限：{error}"));
        }
        limit
    }

    pub fn dismiss_task(&self, task_id: &str) -> bool {
        let task_id = task_id.trim();
        if task_id.is_empty() {
            return false;
        }
        let mut runtime = self.runtime.lock();
        runtime.watcher.registry.dismiss(task_id)
    }

    pub fn snapshot(&self) -> ContinuationSnapshot {
        self.runtime.lock().snapshot()
    }
}

impl Default for ContinuationService {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ContinuationService {
    fn drop(&mut self) {
        self.stop();
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
