//! JSONL event parsing for Codex CLI streaming output.
//!
//! The Codex CLI outputs events as JSONL when run with `--json`.
//! The first event is `thread.started` containing the thread/session ID.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A parsed event from the Codex CLI JSONL stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CodexEvent {
    /// Thread started - first event, contains the session/thread ID.
    #[serde(rename = "thread.started")]
    ThreadStarted {
        thread_id: Uuid,
    },

    /// A new turn (user prompt + agent response cycle) has started.
    #[serde(rename = "turn.started")]
    TurnStarted {},

    /// An item (message, reasoning, tool call, etc.) has been completed.
    #[serde(rename = "item.completed")]
    ItemCompleted {
        item: CompletedItem,
    },

    /// The current turn has completed.
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        #[serde(default)]
        usage: Option<UsageStats>,
    },

    /// Background event (e.g., file watching, indexing).
    #[serde(rename = "background_event")]
    BackgroundEvent {
        #[serde(default)]
        event: Option<String>,
        #[serde(flatten)]
        extra: serde_json::Value,
    },

    /// Prompt sent to the agent.
    #[serde(rename = "prompt.sent")]
    PromptSent {
        prompt: String,
    },

    /// Unknown event type - we capture these to avoid breaking on new event types.
    #[serde(other)]
    Unknown,
}

impl CodexEvent {
    /// Parse a JSONL line into a CodexEvent.
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }

    /// Extract the session/thread ID if this is a thread.started event.
    pub fn session_id(&self) -> Option<Uuid> {
        match self {
            CodexEvent::ThreadStarted { thread_id } => Some(*thread_id),
            _ => None,
        }
    }

    /// Check if this event indicates the turn is complete.
    pub fn is_turn_completed(&self) -> bool {
        matches!(self, CodexEvent::TurnCompleted { .. })
    }

    /// Get the completed item if this is an item.completed event.
    pub fn item(&self) -> Option<&CompletedItem> {
        match self {
            CodexEvent::ItemCompleted { item } => Some(item),
            _ => None,
        }
    }
}

/// A completed item from the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedItem {
    /// Unique ID for this item.
    pub id: String,

    /// Type of item (reasoning, agent_message, tool_call, tool_output, etc.)
    #[serde(rename = "type")]
    pub item_type: String,

    /// Text content (for reasoning and agent_message types).
    #[serde(default)]
    pub text: Option<String>,

    /// Tool name (for tool_call type).
    #[serde(default)]
    pub name: Option<String>,

    /// Tool arguments (for tool_call type).
    #[serde(default)]
    pub arguments: Option<String>,

    /// Tool call ID (for tool_call and tool_output types).
    #[serde(default)]
    pub call_id: Option<String>,

    /// Tool output (for tool_output type).
    #[serde(default)]
    pub output: Option<String>,

    /// Additional fields we don't explicitly model.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl CompletedItem {
    /// Check if this is a reasoning item.
    pub fn is_reasoning(&self) -> bool {
        self.item_type == "reasoning"
    }

    /// Check if this is an agent message.
    pub fn is_agent_message(&self) -> bool {
        self.item_type == "agent_message"
    }

    /// Check if this is a tool call.
    pub fn is_tool_call(&self) -> bool {
        self.item_type == "tool_call"
    }

    /// Check if this is tool output.
    pub fn is_tool_output(&self) -> bool {
        self.item_type == "tool_output"
    }
}

/// Token usage statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub cached_input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD_STARTED_JSON: &str =
        r#"{"type":"thread.started","thread_id":"019b8211-cfdc-7b42-aba2-f10cf3236c70"}"#;

    const TURN_STARTED_JSON: &str = r#"{"type":"turn.started"}"#;

    const ITEM_COMPLETED_REASONING_JSON: &str = r#"{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"**Thinking about the task**"}}"#;

    const ITEM_COMPLETED_MESSAGE_JSON: &str =
        r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"OK"}}"#;

    const TURN_COMPLETED_JSON: &str = r#"{"type":"turn.completed","usage":{"input_tokens":4079,"cached_input_tokens":3200,"output_tokens":7}}"#;

    #[test]
    fn test_parse_thread_started() {
        let event = CodexEvent::parse(THREAD_STARTED_JSON).unwrap();
        match event {
            CodexEvent::ThreadStarted { thread_id } => {
                assert_eq!(
                    thread_id.to_string(),
                    "019b8211-cfdc-7b42-aba2-f10cf3236c70"
                );
            }
            _ => panic!("Expected ThreadStarted event, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_turn_started() {
        let event = CodexEvent::parse(TURN_STARTED_JSON).unwrap();
        assert!(matches!(event, CodexEvent::TurnStarted {}));
    }

    #[test]
    fn test_parse_item_completed_reasoning() {
        let event = CodexEvent::parse(ITEM_COMPLETED_REASONING_JSON).unwrap();
        match event {
            CodexEvent::ItemCompleted { item } => {
                assert_eq!(item.id, "item_0");
                assert!(item.is_reasoning());
                assert_eq!(item.text, Some("**Thinking about the task**".to_string()));
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_item_completed_message() {
        let event = CodexEvent::parse(ITEM_COMPLETED_MESSAGE_JSON).unwrap();
        match event {
            CodexEvent::ItemCompleted { item } => {
                assert_eq!(item.id, "item_1");
                assert!(item.is_agent_message());
                assert_eq!(item.text, Some("OK".to_string()));
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_turn_completed() {
        let event = CodexEvent::parse(TURN_COMPLETED_JSON).unwrap();
        match event {
            CodexEvent::TurnCompleted { usage } => {
                let usage = usage.unwrap();
                assert_eq!(usage.input_tokens, Some(4079));
                assert_eq!(usage.output_tokens, Some(7));
            }
            _ => panic!("Expected TurnCompleted event"),
        }
    }

    #[test]
    fn test_session_id_extraction() {
        let event = CodexEvent::parse(THREAD_STARTED_JSON).unwrap();
        let session_id = event.session_id().unwrap();
        assert_eq!(
            session_id.to_string(),
            "019b8211-cfdc-7b42-aba2-f10cf3236c70"
        );
    }

    #[test]
    fn test_unknown_event_type() {
        let json = r#"{"type":"some.future.event","data":{}}"#;
        let event = CodexEvent::parse(json).unwrap();
        assert!(matches!(event, CodexEvent::Unknown));
    }
}
