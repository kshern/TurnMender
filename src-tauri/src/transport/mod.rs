use crate::core::ChannelStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;

#[cfg(target_os = "macos")]
mod mac_desktop_ipc;
#[cfg(target_os = "windows")]
mod windows_desktop_ipc;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalRecovery {
    /// The task does not have a goal, so the caller may use the normal message path.
    NoGoal,
    /// A goal exists and is already active, so no status change was needed.
    AlreadyActive,
    /// A paused goal was switched back to active.
    Resumed,
    /// A goal exists, but its status must not be changed automatically.
    NotRecoverable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GoalState {
    NoGoal,
    Active,
    Paused,
    Other(String),
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error("消息通道不可用")]
    Unavailable,
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
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
    fn recover_goal(&self, task_id: &str) -> Result<GoalRecovery, SendError>;
}

pub fn make_sender() -> Arc<dyn Sender> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(mac_desktop_ipc::MacDesktopIpc::new())
    }
    #[cfg(target_os = "windows")]
    {
        Arc::new(windows_desktop_ipc::WindowsDesktopIpc::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Arc::new(UnsupportedSender)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct UnsupportedSender;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl Sender for UnsupportedSender {
    fn status(&self) -> ChannelStatus {
        ChannelStatus::Unsupported
    }

    fn send(&self, _request: &SendRequest) -> Result<SendReceipt, SendError> {
        Err(SendError::Unsupported)
    }

    fn recover_goal(&self, _task_id: &str) -> Result<GoalRecovery, SendError> {
        Err(SendError::Unsupported)
    }
}

fn goal_from_response(response: &Value) -> Result<GoalState, SendError> {
    if response.get("resultType").and_then(Value::as_str) != Some("success") {
        return Err(SendError::Protocol(response_detail(response)));
    }
    let result = response.get("result");
    let goal = result.and_then(|result| result.get("goal")).or_else(|| {
        result
            .and_then(|result| result.get("result"))
            .and_then(|result| result.get("goal"))
    });
    let Some(goal) = goal else {
        return Err(SendError::Protocol(
            "目标状态响应缺少 goal 字段".to_string(),
        ));
    };
    if goal.is_null() {
        return Ok(GoalState::NoGoal);
    }
    match goal.get("status").and_then(Value::as_str) {
        Some("active") => Ok(GoalState::Active),
        Some("paused") => Ok(GoalState::Paused),
        Some(status) => Ok(GoalState::Other(status.to_string())),
        None => Err(SendError::Protocol(
            "目标状态响应缺少 status 字段".to_string(),
        )),
    }
}

fn response_detail(response: &Value) -> String {
    response
        .get("error")
        .and_then(|error| {
            error
                .as_str()
                .map(ToOwned::to_owned)
                .or_else(|| serde_json::to_string(error).ok())
        })
        .or_else(|| serde_json::to_string(response).ok())
        .unwrap_or_else(|| "未知响应".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_goal_statuses_without_exposing_goal_objective() {
        assert_eq!(
            goal_from_response(&json!({
                "resultType":"success",
                "result":{"goal":null}
            }))
            .unwrap(),
            GoalState::NoGoal
        );
        assert_eq!(
            goal_from_response(&json!({
                "resultType":"success",
                "result":{"goal":{"status":"active","objective":"secret"}}
            }))
            .unwrap(),
            GoalState::Active
        );
        assert_eq!(
            goal_from_response(&json!({
                "resultType":"success",
                "result":{"result":{"goal":{"status":"paused","objective":"secret"}}}
            }))
            .unwrap(),
            GoalState::Paused
        );
        assert_eq!(
            goal_from_response(&json!({
                "resultType":"success",
                "result":{"goal":{"status":"blocked","objective":"secret"}}
            }))
            .unwrap(),
            GoalState::Other("blocked".into())
        );
    }
}
