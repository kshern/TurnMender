use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CAPACITY_MESSAGE: &str = "Selected model is at capacity";
pub const REMOTE_COMPACT_CAPACITY_MESSAGE: &str =
    "Error running remote compact task: Selected model is at capacity";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LastAgentMessageState {
    ExplicitlyEmpty,
    Present,
    Missing,
}

impl LastAgentMessageState {
    pub fn from_payload(payload: &Value) -> Self {
        match payload.get("last_agent_message") {
            Some(Value::String(text)) if !text.trim().is_empty() => Self::Present,
            Some(Value::String(_)) | Some(Value::Null) => Self::ExplicitlyEmpty,
            _ => Self::Missing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityDecision {
    Eligible,
    CompletedWithOutput,
    Unknown,
    NotCapacity,
}

pub fn is_capacity_error(message: &str, error_info: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let info = error_info.trim();
    info == "server_overloaded"
        || message.contains(&CAPACITY_MESSAGE.to_ascii_lowercase())
        || message.contains(&REMOTE_COMPACT_CAPACITY_MESSAGE.to_ascii_lowercase())
}

pub fn classify_capacity(
    message: &str,
    error_info: &str,
    last_agent_message: &LastAgentMessageState,
) -> CapacityDecision {
    if !is_capacity_error(message, error_info) {
        return CapacityDecision::NotCapacity;
    }
    match last_agent_message {
        LastAgentMessageState::ExplicitlyEmpty => CapacityDecision::Eligible,
        LastAgentMessageState::Present => CapacityDecision::CompletedWithOutput,
        LastAgentMessageState::Missing => CapacityDecision::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recognizes_capacity_variants() {
        assert!(is_capacity_error(
            "Selected model is at capacity. Please try again",
            ""
        ));
        assert!(is_capacity_error("anything", "server_overloaded"));
        assert!(is_capacity_error(
            "Error running remote compact task: Selected model is at capacity",
            ""
        ));
        assert!(!is_capacity_error("network timeout", ""));
    }

    #[test]
    fn distinguishes_missing_and_empty_final_message() {
        assert_eq!(
            LastAgentMessageState::from_payload(&json!({"last_agent_message": null})),
            LastAgentMessageState::ExplicitlyEmpty
        );
        assert_eq!(
            LastAgentMessageState::from_payload(&json!({"last_agent_message": "done"})),
            LastAgentMessageState::Present
        );
        assert_eq!(
            LastAgentMessageState::from_payload(&json!({})),
            LastAgentMessageState::Missing
        );
    }

    #[test]
    fn only_empty_final_message_is_eligible() {
        assert_eq!(
            classify_capacity(
                CAPACITY_MESSAGE,
                "server_overloaded",
                &LastAgentMessageState::ExplicitlyEmpty
            ),
            CapacityDecision::Eligible
        );
        assert_eq!(
            classify_capacity(
                CAPACITY_MESSAGE,
                "server_overloaded",
                &LastAgentMessageState::Present
            ),
            CapacityDecision::CompletedWithOutput
        );
        assert_eq!(
            classify_capacity(
                CAPACITY_MESSAGE,
                "server_overloaded",
                &LastAgentMessageState::Missing
            ),
            CapacityDecision::Unknown
        );
    }
}
