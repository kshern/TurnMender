use super::{SendError, SendReceipt, SendRequest, Sender};
use crate::core::ChannelStatus;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(1800);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

pub struct MacDesktopIpc;

impl MacDesktopIpc {
    pub fn new() -> Self {
        Self
    }

    fn socket_path() -> PathBuf {
        let root = env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
            .unwrap_or_else(|| PathBuf::from(".codex"));
        root.join("ipc/ipc.sock")
    }

    fn verified_socket() -> Option<PathBuf> {
        let socket = Self::socket_path();
        Self::verified_socket_at(&socket)
    }

    fn verified_socket_at(socket: &Path) -> Option<PathBuf> {
        let metadata = fs::symlink_metadata(socket).ok()?;
        let uid = unsafe { libc::getuid() };
        if metadata.uid() != uid || !metadata.file_type().is_socket() {
            return None;
        }
        let directory = socket.parent()?;
        let directory_metadata = fs::symlink_metadata(directory).ok()?;
        if directory_metadata.uid() != uid
            || !directory_metadata.file_type().is_dir()
            || directory_metadata.mode() & 0o022 != 0
        {
            return None;
        }
        Some(socket.to_path_buf())
    }
}

impl Sender for MacDesktopIpc {
    fn status(&self) -> ChannelStatus {
        if Self::verified_socket().is_some() {
            ChannelStatus::Ready
        } else {
            ChannelStatus::Unavailable
        }
    }

    fn send(&self, request: &SendRequest) -> Result<SendReceipt, SendError> {
        let socket = Self::verified_socket().ok_or(SendError::Unavailable)?;
        let mut client = Client::connect(&socket)?;
        let owner = client.discover_owner(&request.task_id)?;
        client.start_turn(&request.task_id, &owner, &request.message)
    }
}

struct Client {
    stream: UnixStream,
    client_id: String,
}

impl Client {
    fn connect(path: &Path) -> Result<Self, SendError> {
        let stream =
            UnixStream::connect(path).map_err(|error| SendError::Failed(error.to_string()))?;
        stream
            .set_read_timeout(Some(DISCOVERY_TIMEOUT))
            .map_err(|error| SendError::Failed(error.to_string()))?;
        stream
            .set_write_timeout(Some(DISCOVERY_TIMEOUT))
            .map_err(|error| SendError::Failed(error.to_string()))?;
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
                    "Codex 未接受继续消息: {}",
                    Self::response_detail(&response)
                ))
            })?;
        Ok(SendReceipt {
            accepted: true,
            new_turn_id: turn_id.to_string(),
            protocol_version: 1,
        })
    }

    fn request(
        &mut self,
        method: &str,
        version: i64,
        params: Value,
        target: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, SendError> {
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| SendError::Failed(error.to_string()))?;
        self.stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| SendError::Failed(error.to_string()))?;
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
                self.write_frame(&json!({"type":"client-discovery-response", "requestId":discovery_id, "response":{"canHandle":false}}))?;
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
            .map_err(|error| SendError::Failed(error.to_string()))
    }

    fn read_frame(&mut self) -> Result<Value, SendError> {
        let mut header = [0u8; 4];
        self.stream
            .read_exact(&mut header)
            .map_err(|error| SendError::Failed(error.to_string()))?;
        let length = u32::from_le_bytes(header) as usize;
        if length == 0 || length > MAX_FRAME_SIZE {
            return Err(SendError::Protocol("返回消息大小异常".to_string()));
        }
        let mut payload = vec![0u8; length];
        self.stream
            .read_exact(&mut payload)
            .map_err(|error| SendError::Failed(error.to_string()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_status_is_safe_when_socket_missing() {
        let missing =
            std::env::temp_dir().join(format!("turnmender-missing-socket-{}", Uuid::new_v4()));
        assert!(MacDesktopIpc::verified_socket_at(&missing).is_none());
    }

    #[test]
    fn regular_file_is_not_accepted_as_socket() {
        let root = std::env::temp_dir().join(format!("turnmender-fake-socket-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let fake_socket = root.join("ipc.sock");
        fs::write(&fake_socket, b"not a socket").unwrap();

        assert!(MacDesktopIpc::verified_socket_at(&fake_socket).is_none());

        fs::remove_dir_all(root).unwrap();
    }
}
