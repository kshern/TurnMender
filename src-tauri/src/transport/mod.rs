use crate::core::ChannelStatus;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

#[cfg(target_os = "macos")]
mod mac_desktop_ipc;

pub const RETRY_MESSAGE: &str =
    "Continue the previous task. First inspect the current workspace and review the work already completed to avoid repeating any actions, then resume from where it was interrupted.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendRequest {
    pub task_id: String,
    pub failed_turn_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendReceipt {
    pub accepted: bool,
    pub new_turn_id: String,
    pub protocol_version: i64,
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error("消息通道不可用")]
    Unavailable,
    #[cfg(not(target_os = "macos"))]
    #[error("消息通道暂不支持当前平台")]
    Unsupported,
    #[error("任务不存在或没有持有者")]
    NotFound,
    #[error("消息通道协议不兼容: {0}")]
    Protocol(String),
    #[error("消息通道发送失败: {0}")]
    Failed(String),
}

pub trait Sender: Send + Sync {
    fn status(&self) -> ChannelStatus;
    fn send(&self, request: &SendRequest) -> Result<SendReceipt, SendError>;
}

pub fn make_sender() -> Arc<dyn Sender> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(mac_desktop_ipc::MacDesktopIpc::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(UnsupportedSender)
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedSender;

#[cfg(not(target_os = "macos"))]
impl Sender for UnsupportedSender {
    fn status(&self) -> ChannelStatus {
        ChannelStatus::Unsupported
    }

    fn send(&self, _request: &SendRequest) -> Result<SendReceipt, SendError> {
        Err(SendError::Unsupported)
    }
}
