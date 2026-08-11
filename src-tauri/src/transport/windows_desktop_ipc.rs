use super::{
    goal_from_response, GoalRecovery, GoalState, SendError, SendReceipt, SendRequest, Sender,
};
use crate::core::ChannelStatus;
use named_pipe::PipeClient;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::Duration;
use uuid::Uuid;

const MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(1800);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const PIPE_NAME: &str = r"\\.\pipe\codex-ipc";

pub struct WindowsDesktopIpc;

impl WindowsDesktopIpc {
    pub fn new() -> Self {
        Self
    }

    fn pipe_name() -> &'static str {
        PIPE_NAME
    }

    fn connect(timeout: Duration) -> Result<Client, SendError> {
        Client::connect(Self::pipe_name(), timeout)
    }
}

impl Sender for WindowsDesktopIpc {
    fn status(&self) -> ChannelStatus {
        if PipeClient::connect_ms(Self::pipe_name(), 25).is_ok() {
            ChannelStatus::Ready
        } else {
            ChannelStatus::Unavailable
        }
    }

    fn send(&self, request: &SendRequest) -> Result<SendReceipt, SendError> {
        let mut client = Self::connect(DISCOVERY_TIMEOUT)?;
        let owner = client.discover_owner(&request.task_id)?;
        client.start_turn(&request.task_id, &owner, &request.message)
    }

    fn recover_goal(&self, task_id: &str) -> Result<GoalRecovery, SendError> {
        let mut client = Self::connect(DISCOVERY_TIMEOUT)?;
        let owner = client.discover_owner(task_id)?;
        client.recover_goal(task_id, &owner)
    }
}

struct Client {
    stream: PipeClient,
    client_id: String,
}

impl Client {
    fn connect(pipe_name: &str, timeout: Duration) -> Result<Self, SendError> {
        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        let stream = PipeClient::connect_ms(pipe_name, timeout_ms).map_err(map_connect_error)?;
        let mut client = Self {
            stream,
            client_id: "initializing-client".to_string(),
        };
        let response = client.request(
            "initialize",
            0,
            json!({"clientType":"turnmender"}),
            None,
            DISCOVERY_TIMEOUT,
        )?;
        if response.get("resultType").and_then(Value::as_str) != Some("success") {
            return Err(SendError::Protocol(Self::response_detail(&response)));
        }
        client.client_id = response
            .get("result")
            .and_then(|value| value.get("clientId"))
            .and_then(Value::as_str)
            .ok_or_else(|| SendError::Protocol("初始化没有返回 clientId".to_string()))?
            .to_string();
        Ok(client)
    }

    fn discover_owner(&mut self, task_id: &str) -> Result<String, SendError> {
        let response = self.request(
            "thread-owner-discovery",
            1,
            json!({"hostId":"local", "conversationId":task_id}),
            None,
            DISCOVERY_TIMEOUT,
        )?;
        if response.get("resultType").and_then(Value::as_str) != Some("success") {
            return Err(SendError::NotFound);
        }
        response
            .get("handledByClientId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or(SendError::NotFound)
    }

    fn start_turn(
        &mut self,
        task_id: &str,
        owner: &str,
        message: &str,
    ) -> Result<SendReceipt, SendError> {
        let response = self.request(
            "thread-follower-start-turn",
            1,
            json!({
                "conversationId": task_id,
                "turnStartParams": {
                    "input": [{"type":"text", "text":message, "text_elements":[]}],
                    "inheritThreadSettings": true,
                    "useAppServerPermissionDefault": true
                },
                "mcpAppModelContextAttachments": []
            }),
            Some(owner),
            REQUEST_TIMEOUT,
        )?;
        let turn_id = response
            .get("resultType")
            .and_then(Value::as_str)
            .filter(|value| *value == "success")
            .and_then(|_| response.get("method").and_then(Value::as_str))
            .filter(|value| *value == "thread-follower-start-turn")
            .and_then(|_| response.get("result"))
            .and_then(|value| value.get("result"))
            .and_then(|value| value.get("turn"))
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                SendError::Protocol(format!(
                    "Codex 未接收继续消息: {}",
                    Self::response_detail(&response)
                ))
            })?;
        Ok(SendReceipt {
            accepted: true,
            new_turn_id: turn_id.to_string(),
            protocol_version: 1,
        })
    }

    fn recover_goal(&mut self, task_id: &str, owner: &str) -> Result<GoalRecovery, SendError> {
        let response = self.request(
            "thread/goal/get",
            2,
            json!({"threadId":task_id}),
            Some(owner),
            REQUEST_TIMEOUT,
        )?;
        match goal_from_response(&response)? {
            GoalState::Paused => {}
            GoalState::NoGoal => return Ok(GoalRecovery::NoGoal),
            GoalState::Active => return Ok(GoalRecovery::AlreadyActive),
            GoalState::Other(status) => return Ok(GoalRecovery::NotRecoverable(status)),
        }

        let response = self.request(
            "thread/goal/set",
            2,
            json!({"threadId":task_id, "status":"active"}),
            Some(owner),
            REQUEST_TIMEOUT,
        )?;
        match goal_from_response(&response)? {
            GoalState::Active => Ok(GoalRecovery::Resumed),
            state => Err(SendError::Protocol(format!(
                "目标恢复后状态异常: {state:?}"
            ))),
        }
    }

    fn request(
        &mut self,
        method: &str,
        version: i64,
        params: Value,
        target: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, SendError> {
        self.stream.set_read_timeout(Some(timeout));
        self.stream.set_write_timeout(Some(timeout));
        let request_id = Uuid::new_v4().to_string();
        let mut request = json!({
            "type":"request", "requestId":request_id, "sourceClientId":self.client_id,
            "version":version, "method":method, "params":params,
            "timeoutMs":timeout.as_millis()
        });
        if let Some(target) = target {
            request["targetClientId"] = Value::String(target.to_string());
        }
        self.write_frame(&request)?;
        loop {
            let response = self.read_frame()?;
            if response.get("type").and_then(Value::as_str) == Some("client-discovery-request") {
                let discovery_id = response
                    .get("requestId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.write_frame(&json!({
                    "type":"client-discovery-response",
                    "requestId":discovery_id,
                    "response":{"canHandle":false}
                }))?;
                continue;
            }
            if response.get("type").and_then(Value::as_str) == Some("response")
                && response.get("requestId").and_then(Value::as_str) == Some(request_id.as_str())
            {
                return Ok(response);
            }
        }
    }

    fn write_frame(&mut self, value: &Value) -> Result<(), SendError> {
        let payload =
            serde_json::to_vec(value).map_err(|error| SendError::Protocol(error.to_string()))?;
        if payload.is_empty() || payload.len() > MAX_FRAME_SIZE {
            return Err(SendError::Protocol("消息大小异常".to_string()));
        }
        let length = (payload.len() as u32).to_le_bytes();
        self.stream
            .write_all(&length)
            .and_then(|_| self.stream.write_all(&payload))
            .and_then(|_| self.stream.flush())
            .map_err(map_io_error)
    }

    fn read_frame(&mut self) -> Result<Value, SendError> {
        let mut header = [0u8; 4];
        self.stream.read_exact(&mut header).map_err(map_io_error)?;
        let length = u32::from_le_bytes(header) as usize;
        if length == 0 || length > MAX_FRAME_SIZE {
            return Err(SendError::Protocol("返回消息大小异常".to_string()));
        }
        let mut payload = vec![0u8; length];
        self.stream.read_exact(&mut payload).map_err(map_io_error)?;
        serde_json::from_slice(&payload).map_err(|error| SendError::Protocol(error.to_string()))
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
}

fn map_io_error(error: std::io::Error) -> SendError {
    match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::TimedOut => SendError::Unavailable,
        _ => SendError::Failed(error.to_string()),
    }
}

fn map_connect_error(_: std::io::Error) -> SendError {
    SendError::Unavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_name_targets_the_codex_local_pipe() {
        assert_eq!(WindowsDesktopIpc::pipe_name(), r"\\.\pipe\codex-ipc");
    }
}
