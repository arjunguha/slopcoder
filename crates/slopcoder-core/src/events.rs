//! JSONL event parsing for agent streaming output.
//!
//! The Codex CLI outputs events as JSONL when run with `--json`.
//! The Claude CLI outputs JSONL when run with `--print --output-format stream-json`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A parsed event from an agent JSONL stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Session started - first event, contains the session ID.
    #[serde(rename = "session.started", alias = "thread.started")]
    SessionStarted {
        #[serde(alias = "thread_id")]
        session_id: Uuid,
    },

    /// A new turn (user prompt + agent response cycle) has started.
    #[serde(rename = "turn.started")]
    TurnStarted {},

    /// An item (message, reasoning, tool call, etc.) has been completed.
    #[serde(rename = "item.completed")]
    ItemCompleted { item: CompletedItem },

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
    PromptSent { prompt: String },

    /// Unknown event type - we capture these to avoid breaking on new event types.
    #[serde(other)]
    Unknown,
}

impl AgentEvent {
    /// Parse a JSONL line into an AgentEvent (Codex format).
    pub fn parse_codex(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }

    /// Parse a JSONL line into AgentEvents (Claude format).
    pub fn parse_claude(line: &str) -> Result<Vec<Self>, serde_json::Error> {
        let parsed: ClaudeStreamEvent = serde_json::from_str(line)?;
        Ok(parsed.into_agent_events())
    }

    /// Parse a JSONL line into AgentEvents (Cursor format).
    pub fn parse_cursor(line: &str) -> Result<Vec<Self>, serde_json::Error> {
        let parsed: CursorStreamEvent = serde_json::from_str(line)?;
        Ok(parsed.into_agent_events())
    }

    /// Parse a JSONL line into AgentEvents (OpenCode format).
    pub fn parse_opencode(line: &str) -> Result<Vec<Self>, serde_json::Error> {
        let parsed: OpencodeStreamEvent = serde_json::from_str(line)?;
        Ok(parsed.into_agent_events())
    }

    /// Parse a JSONL line into AgentEvents (Gemini format).
    pub fn parse_gemini(line: &str) -> Result<Vec<Self>, serde_json::Error> {
        let parsed: GeminiStreamEvent = serde_json::from_str(line)?;
        Ok(parsed.into_agent_events())
    }

    /// Extract the session ID if this is a session.started event.
    pub fn session_id(&self) -> Option<Uuid> {
        match self {
            AgentEvent::SessionStarted { session_id } => Some(*session_id),
            _ => None,
        }
    }

    /// Extract the OpenCode session ID (UUID and original string) if this is a session started event.
    /// Returns (generated_uuid, original_session_string).
    pub fn opencode_session_id(&self) -> Option<(Uuid, String)> {
        match self {
            AgentEvent::BackgroundEvent { event, extra } => {
                // OpenCode stores session info in BackgroundEvent
                if event.as_deref() == Some("opencode_session") {
                    if let Some(session_str) = extra.get("session_string").and_then(|v| v.as_str())
                    {
                        let uuid = opencode_session_string_to_uuid(session_str);
                        return Some((uuid, session_str.to_string()));
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Check if this event indicates the turn is complete.
    pub fn is_turn_completed(&self) -> bool {
        matches!(self, AgentEvent::TurnCompleted { .. })
    }

    /// Get the completed item if this is an item.completed event.
    pub fn item(&self) -> Option<&CompletedItem> {
        match self {
            AgentEvent::ItemCompleted { item } => Some(item),
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
    /// Note: This uses a custom serializer to handle non-object values gracefully.
    /// The `#[serde(flatten)]` attribute only works with maps/objects, so we skip
    /// serialization for non-object values (strings, arrays, etc.) to avoid errors.
    #[serde(flatten, serialize_with = "serialize_extra_if_object")]
    pub extra: serde_json::Value,
}

/// Custom serializer for `extra` that only serializes if it's an object.
/// Non-object values (null, string, array, etc.) are skipped during serialization
/// because `#[serde(flatten)]` only works with maps/objects.
fn serialize_extra_if_object<S>(value: &serde_json::Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    match value {
        serde_json::Value::Object(map) => {
            let mut ser_map = serializer.serialize_map(Some(map.len()))?;
            for (k, v) in map {
                ser_map.serialize_entry(k, v)?;
            }
            ser_map.end()
        }
        // For non-object values, serialize an empty map (effectively skipping)
        _ => serializer.serialize_map(Some(0))?.end(),
    }
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeStreamEvent {
    #[serde(rename = "system")]
    System {
        #[serde(default)]
        #[allow(dead_code)]
        subtype: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
    },
    #[serde(rename = "assistant")]
    Assistant {
        message: ClaudeMessage,
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<Uuid>,
    },
    #[serde(rename = "user")]
    User {
        message: ClaudeUserMessage,
        #[serde(default)]
        tool_use_result: Option<serde_json::Value>,
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<Uuid>,
    },
    #[serde(rename = "result")]
    Result {
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<Uuid>,
        #[serde(default)]
        usage: Option<ClaudeUsage>,
    },
    #[serde(other)]
    Unknown,
}

impl ClaudeStreamEvent {
    fn into_agent_events(self) -> Vec<AgentEvent> {
        match self {
            ClaudeStreamEvent::System { session_id, .. } => {
                if let Some(session_id) = session_id {
                    vec![AgentEvent::SessionStarted { session_id }]
                } else {
                    vec![AgentEvent::Unknown]
                }
            }
            ClaudeStreamEvent::Assistant { message, .. } => message.into_events(),
            ClaudeStreamEvent::User {
                message,
                tool_use_result,
                ..
            } => message.into_events(tool_use_result),
            ClaudeStreamEvent::Result { usage, .. } => vec![AgentEvent::TurnCompleted {
                usage: usage.map(|u| UsageStats {
                    input_tokens: u.input_tokens,
                    cached_input_tokens: u.cache_read_input_tokens,
                    output_tokens: u.output_tokens,
                }),
            }],
            ClaudeStreamEvent::Unknown => vec![AgentEvent::Unknown],
        }
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    id: String,
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Deserialize)]
struct ClaudeContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[serde(default)]
    tool_use_id: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

impl ClaudeMessage {
    fn into_events(self) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        let mut text = String::new();

        for block in self.content {
            match block.kind.as_str() {
                "text" => {
                    if let Some(t) = block.text {
                        text.push_str(&t);
                    }
                }
                "tool_use" => {
                    let call_id = block.id.clone();
                    let arguments = block
                        .input
                        .as_ref()
                        .and_then(|value| serde_json::to_string(value).ok());
                    events.push(AgentEvent::ItemCompleted {
                        item: CompletedItem {
                            id: block.id.unwrap_or_else(|| self.id.clone()),
                            item_type: "tool_call".to_string(),
                            text: None,
                            name: block.name,
                            arguments,
                            call_id,
                            output: None,
                            extra: serde_json::Value::Null,
                        },
                    });
                }
                _ => {}
            }
        }

        if !text.is_empty() {
            events.push(AgentEvent::ItemCompleted {
                item: CompletedItem {
                    id: self.id,
                    item_type: "agent_message".to_string(),
                    text: Some(text),
                    name: None,
                    arguments: None,
                    call_id: None,
                    output: None,
                    extra: serde_json::Value::Null,
                },
            });
        }

        if events.is_empty() {
            events.push(AgentEvent::Unknown);
        }

        events
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeUserMessage {
    content: Vec<ClaudeContent>,
}

impl ClaudeUserMessage {
    fn into_events(self, tool_use_result: Option<serde_json::Value>) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        for block in self.content {
            if block.kind == "tool_result" {
                events.push(AgentEvent::ItemCompleted {
                    item: CompletedItem {
                        id: block
                            .tool_use_id
                            .clone()
                            .unwrap_or_else(|| "tool_result".to_string()),
                        item_type: "tool_output".to_string(),
                        text: None,
                        name: None,
                        arguments: None,
                        call_id: block.tool_use_id,
                        output: block.content.or(block.text),
                        extra: tool_use_result.clone().unwrap_or(serde_json::Value::Null),
                    },
                });
            }
        }

        if events.is_empty() {
            events.push(AgentEvent::Unknown);
        }

        events
    }
}

#[derive(Debug, Deserialize)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CursorStreamEvent {
    #[serde(rename = "system")]
    System {
        #[serde(default)]
        #[allow(dead_code)]
        subtype: Option<String>,
        #[serde(default)]
        session_id: Option<Uuid>,
        #[serde(flatten)]
        #[allow(dead_code)]
        extra: serde_json::Value,
    },
    #[serde(rename = "assistant")]
    Assistant {
        message: CursorMessage,
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<Uuid>,
        #[serde(flatten)]
        #[allow(dead_code)]
        extra: serde_json::Value,
    },
    #[serde(rename = "user")]
    User {
        message: CursorUserMessage,
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<Uuid>,
        #[serde(flatten)]
        #[allow(dead_code)]
        extra: serde_json::Value,
    },
    #[serde(rename = "result")]
    Result {
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<Uuid>,
        #[serde(default)]
        #[allow(dead_code)]
        is_error: Option<bool>,
        #[serde(flatten)]
        #[allow(dead_code)]
        extra: serde_json::Value,
    },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        #[allow(dead_code)]
        subtype: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        #[allow(dead_code)]
        session_id: Option<Uuid>,
        #[serde(flatten)]
        #[allow(dead_code)]
        extra: serde_json::Value,
    },
    #[serde(other)]
    Unknown,
}

impl CursorStreamEvent {
    fn into_agent_events(self) -> Vec<AgentEvent> {
        match self {
            CursorStreamEvent::System { session_id, .. } => {
                if let Some(session_id) = session_id {
                    vec![AgentEvent::SessionStarted { session_id }]
                } else {
                    vec![AgentEvent::Unknown]
                }
            }
            CursorStreamEvent::Assistant { message, .. } => message.into_events(),
            CursorStreamEvent::User { .. } => {
                // User messages are typically prompts, we can ignore or mark as prompt.sent
                vec![AgentEvent::Unknown]
            }
            CursorStreamEvent::Result { is_error, .. } => {
                vec![AgentEvent::TurnCompleted { usage: None }]
            }
            CursorStreamEvent::Thinking { text, .. } => {
                if let Some(text) = text {
                    vec![AgentEvent::ItemCompleted {
                        item: CompletedItem {
                            id: uuid::Uuid::new_v4().to_string(),
                            item_type: "reasoning".to_string(),
                            text: Some(text),
                            name: None,
                            arguments: None,
                            call_id: None,
                            output: None,
                            extra: serde_json::Value::Null,
                        },
                    }]
                } else {
                    vec![AgentEvent::Unknown]
                }
            }
            CursorStreamEvent::Unknown => vec![AgentEvent::Unknown],
        }
    }
}

#[derive(Debug, Deserialize)]
struct CursorMessage {
    #[serde(default)]
    role: Option<String>,
    content: Vec<CursorContent>,
}

#[derive(Debug, Deserialize)]
struct CursorContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[serde(default)]
    tool_use_id: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(flatten)]
    #[allow(dead_code)]
    extra: serde_json::Value,
}

impl CursorMessage {
    fn into_events(self) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        let mut text = String::new();

        for block in self.content {
            match block.kind.as_str() {
                "text" => {
                    if let Some(t) = block.text {
                        text.push_str(&t);
                    }
                }
                "tool_use" => {
                    let call_id = block.id.clone();
                    let arguments = block
                        .input
                        .as_ref()
                        .and_then(|value| serde_json::to_string(value).ok());
                    events.push(AgentEvent::ItemCompleted {
                        item: CompletedItem {
                            id: block.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            item_type: "tool_call".to_string(),
                            text: None,
                            name: block.name,
                            arguments,
                            call_id,
                            output: None,
                            extra: serde_json::Value::Null,
                        },
                    });
                }
                _ => {}
            }
        }

        if !text.is_empty() {
            events.push(AgentEvent::ItemCompleted {
                item: CompletedItem {
                    id: uuid::Uuid::new_v4().to_string(),
                    item_type: "agent_message".to_string(),
                    text: Some(text),
                    name: None,
                    arguments: None,
                    call_id: None,
                    output: None,
                    extra: serde_json::Value::Null,
                },
            });
        }

        if events.is_empty() {
            events.push(AgentEvent::Unknown);
        }

        events
    }
}

#[derive(Debug, Deserialize)]
struct CursorUserMessage {
    #[serde(default)]
    role: Option<String>,
    content: Vec<CursorContent>,
}

/// Generate a deterministic UUID from an opencode session string.
fn opencode_session_string_to_uuid(session_string: &str) -> Uuid {
    // Use UUID v5 (SHA-1 based) with a custom namespace
    let namespace = Uuid::NAMESPACE_OID;
    Uuid::new_v5(&namespace, session_string.as_bytes())
}

/// OpenCode stream event types.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OpencodeStreamEvent {
    #[serde(rename = "step_start")]
    StepStart {
        #[serde(default, rename = "sessionID")]
        session_id: Option<String>,
        #[serde(default)]
        part: Option<serde_json::Value>,
    },
    #[serde(rename = "text")]
    Text {
        #[serde(default, rename = "sessionID")]
        session_id: Option<String>,
        #[serde(default)]
        part: Option<OpencodeTextPart>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default, rename = "sessionID")]
        session_id: Option<String>,
        #[serde(default)]
        part: Option<OpencodeToolPart>,
    },
    #[serde(rename = "step_finish")]
    StepFinish {
        #[serde(default, rename = "sessionID")]
        session_id: Option<String>,
        #[serde(default)]
        part: Option<OpencodeStepFinishPart>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct OpencodeTextPart {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpencodeToolPart {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "callID")]
    call_id: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    state: Option<OpencodeToolState>,
}

#[derive(Debug, Deserialize)]
struct OpencodeToolState {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    #[serde(default)]
    output: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpencodeStepFinishPart {
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    tokens: Option<OpencodeTokens>,
}

#[derive(Debug, Deserialize)]
struct OpencodeTokens {
    #[serde(default)]
    input: Option<u64>,
    #[serde(default)]
    output: Option<u64>,
    #[serde(default)]
    reasoning: Option<u64>,
}

impl OpencodeStreamEvent {
    fn into_agent_events(self) -> Vec<AgentEvent> {
        match self {
            OpencodeStreamEvent::StepStart { session_id, .. } => {
                // Emit SessionStarted with UUID, plus BackgroundEvent with original string
                if let Some(session_str) = session_id {
                    let uuid = opencode_session_string_to_uuid(&session_str);
                    vec![
                        AgentEvent::SessionStarted { session_id: uuid },
                        AgentEvent::BackgroundEvent {
                            event: Some("opencode_session".to_string()),
                            extra: serde_json::json!({
                                "session_string": session_str
                            }),
                        },
                    ]
                } else {
                    vec![AgentEvent::TurnStarted {}]
                }
            }
            OpencodeStreamEvent::Text { part, .. } => {
                if let Some(part) = part {
                    if let Some(text) = part.text {
                        return vec![AgentEvent::ItemCompleted {
                            item: CompletedItem {
                                id: part.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                                item_type: "agent_message".to_string(),
                                text: Some(text),
                                name: None,
                                arguments: None,
                                call_id: None,
                                output: None,
                                extra: serde_json::Value::Null,
                            },
                        }];
                    }
                }
                vec![AgentEvent::Unknown]
            }
            OpencodeStreamEvent::ToolUse { part, .. } => {
                if let Some(part) = part {
                    let arguments = part
                        .state
                        .as_ref()
                        .and_then(|s| s.input.as_ref())
                        .and_then(|v| serde_json::to_string(v).ok());
                    let output = part.state.as_ref().and_then(|s| s.output.clone());

                    return vec![AgentEvent::ItemCompleted {
                        item: CompletedItem {
                            id: part.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            item_type: "tool_call".to_string(),
                            text: None,
                            name: part.tool,
                            arguments,
                            call_id: part.call_id,
                            output,
                            extra: serde_json::Value::Null,
                        },
                    }];
                }
                vec![AgentEvent::Unknown]
            }
            OpencodeStreamEvent::StepFinish { part, .. } => {
                let usage = part.and_then(|p| {
                    p.tokens.map(|t| UsageStats {
                        input_tokens: t.input,
                        cached_input_tokens: None,
                        output_tokens: t.output,
                    })
                });
                vec![AgentEvent::TurnCompleted { usage }]
            }
            OpencodeStreamEvent::Unknown => vec![AgentEvent::Unknown],
        }
    }
}

/// Gemini stream event types.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum GeminiStreamEvent {
    #[serde(rename = "init")]
    Init {
        session_id: Uuid,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    #[serde(rename = "message")]
    Message {
        role: String,
        content: String,
        #[serde(default)]
        delta: bool,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        tool_name: String,
        tool_id: String,
        parameters: serde_json::Value,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_id: String,
        status: String,
        output: Option<String>,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    #[serde(rename = "result")]
    Result {
        status: String,
        #[serde(default)]
        stats: Option<GeminiStats>,
        #[serde(flatten)]
        extra: serde_json::Value,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct GeminiStats {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default, rename = "cached")]
    cached_tokens: Option<u64>,
    #[serde(flatten)]
    extra: serde_json::Value,
}

impl GeminiStreamEvent {
    fn into_agent_events(self) -> Vec<AgentEvent> {
        match self {
            GeminiStreamEvent::Init { session_id, .. } => {
                vec![AgentEvent::SessionStarted { session_id }]
            }
            GeminiStreamEvent::Message { role, content, .. } => {
                if role == "assistant" {
                    vec![AgentEvent::ItemCompleted {
                        item: CompletedItem {
                            id: uuid::Uuid::new_v4().to_string(),
                            item_type: "agent_message".to_string(),
                            text: Some(content),
                            name: None,
                            arguments: None,
                            call_id: None,
                            output: None,
                            extra: serde_json::Value::Null,
                        },
                    }]
                } else {
                    // User messages are typically prompts or local echos
                    vec![AgentEvent::Unknown]
                }
            }
            GeminiStreamEvent::ToolUse {
                tool_name,
                tool_id,
                parameters,
                ..
            } => {
                let arguments = serde_json::to_string(&parameters).ok();
                vec![AgentEvent::ItemCompleted {
                    item: CompletedItem {
                        id: uuid::Uuid::new_v4().to_string(),
                        item_type: "tool_call".to_string(),
                        text: None,
                        name: Some(tool_name),
                        arguments,
                        call_id: Some(tool_id),
                        output: None,
                        extra: serde_json::Value::Null,
                    },
                }]
            }
            GeminiStreamEvent::ToolResult {
                tool_id, output, ..
            } => vec![AgentEvent::ItemCompleted {
                item: CompletedItem {
                    id: uuid::Uuid::new_v4().to_string(),
                    item_type: "tool_output".to_string(),
                    text: None,
                    name: None,
                    arguments: None,
                    call_id: Some(tool_id),
                    output,
                    extra: serde_json::Value::Null,
                },
            }],
            GeminiStreamEvent::Result { stats, .. } => {
                let usage = stats.map(|s| UsageStats {
                    input_tokens: s.input_tokens,
                    cached_input_tokens: s.cached_tokens,
                    output_tokens: s.output_tokens,
                });
                vec![AgentEvent::TurnCompleted { usage }]
            }
            GeminiStreamEvent::Unknown => vec![AgentEvent::Unknown],
        }
    }
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

    const CLAUDE_SYSTEM_JSON: &str =
        r#"{"type":"system","subtype":"init","session_id":"6c0b0f60-d9b0-4ee7-9f12-6de09fbfc6d5"}"#;
    const CLAUDE_ASSISTANT_TEXT_JSON: &str =
        r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"text","text":"Hi"}]}}"#;
    const CLAUDE_ASSISTANT_TOOL_JSON: &str = r#"{"type":"assistant","message":{"id":"msg_tool","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}]}}"#;
    const CLAUDE_TOOL_RESULT_JSON: &str = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok","is_error":false}]},"tool_use_result":{"stdout":"ok","stderr":""}}"#;
    const CLAUDE_RESULT_JSON: &str = r#"{"type":"result","usage":{"input_tokens":3,"cache_read_input_tokens":10,"output_tokens":5}}"#;

    #[test]
    fn test_parse_thread_started() {
        let event = AgentEvent::parse_codex(THREAD_STARTED_JSON).unwrap();
        match event {
            AgentEvent::SessionStarted { session_id } => {
                assert_eq!(
                    session_id.to_string(),
                    "019b8211-cfdc-7b42-aba2-f10cf3236c70"
                );
            }
            _ => panic!("Expected ThreadStarted event, got {:?}", event),
        }
    }

    #[test]
    fn test_parse_turn_started() {
        let event = AgentEvent::parse_codex(TURN_STARTED_JSON).unwrap();
        assert!(matches!(event, AgentEvent::TurnStarted {}));
    }

    #[test]
    fn test_parse_item_completed_reasoning() {
        let event = AgentEvent::parse_codex(ITEM_COMPLETED_REASONING_JSON).unwrap();
        match event {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.id, "item_0");
                assert!(item.is_reasoning());
                assert_eq!(item.text, Some("**Thinking about the task**".to_string()));
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_item_completed_message() {
        let event = AgentEvent::parse_codex(ITEM_COMPLETED_MESSAGE_JSON).unwrap();
        match event {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.id, "item_1");
                assert!(item.is_agent_message());
                assert_eq!(item.text, Some("OK".to_string()));
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_turn_completed() {
        let event = AgentEvent::parse_codex(TURN_COMPLETED_JSON).unwrap();
        match event {
            AgentEvent::TurnCompleted { usage } => {
                let usage = usage.unwrap();
                assert_eq!(usage.input_tokens, Some(4079));
                assert_eq!(usage.output_tokens, Some(7));
            }
            _ => panic!("Expected TurnCompleted event"),
        }
    }

    #[test]
    fn test_session_id_extraction() {
        let event = AgentEvent::parse_codex(THREAD_STARTED_JSON).unwrap();
        let session_id = event.session_id().unwrap();
        assert_eq!(
            session_id.to_string(),
            "019b8211-cfdc-7b42-aba2-f10cf3236c70"
        );
    }

    #[test]
    fn test_unknown_event_type() {
        let json = r#"{"type":"some.future.event","data":{}}"#;
        let event = AgentEvent::parse_codex(json).unwrap();
        assert!(matches!(event, AgentEvent::Unknown));
    }

    #[test]
    fn test_parse_claude_system() {
        let events = AgentEvent::parse_claude(CLAUDE_SYSTEM_JSON).unwrap();
        assert!(matches!(events[0], AgentEvent::SessionStarted { .. }));
    }

    #[test]
    fn test_parse_claude_assistant_message() {
        let events = AgentEvent::parse_claude(CLAUDE_ASSISTANT_TEXT_JSON).unwrap();
        match &events[0] {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.id, "msg_1");
                assert_eq!(item.item_type, "agent_message");
                assert_eq!(item.text, Some("Hi".to_string()));
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_claude_tool_call() {
        let events = AgentEvent::parse_claude(CLAUDE_ASSISTANT_TOOL_JSON).unwrap();
        match &events[0] {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.item_type, "tool_call");
                assert_eq!(item.call_id.as_deref(), Some("toolu_1"));
                assert_eq!(item.name.as_deref(), Some("Bash"));
            }
            _ => panic!("Expected tool call event"),
        }
    }

    #[test]
    fn test_parse_claude_tool_result() {
        let events = AgentEvent::parse_claude(CLAUDE_TOOL_RESULT_JSON).unwrap();
        match &events[0] {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.item_type, "tool_output");
                assert_eq!(item.call_id.as_deref(), Some("toolu_1"));
                assert_eq!(item.output.as_deref(), Some("ok"));
            }
            _ => panic!("Expected tool output event"),
        }
    }

    #[test]
    fn test_parse_claude_result_usage() {
        let events = AgentEvent::parse_claude(CLAUDE_RESULT_JSON).unwrap();
        match &events[0] {
            AgentEvent::TurnCompleted { usage } => {
                let usage = usage.as_ref().unwrap();
                assert_eq!(usage.input_tokens, Some(3));
                assert_eq!(usage.cached_input_tokens, Some(10));
                assert_eq!(usage.output_tokens, Some(5));
            }
            _ => panic!("Expected TurnCompleted event"),
        }
    }

    // OpenCode event parsing tests
    const OPENCODE_STEP_START_JSON: &str = r#"{"type":"step_start","timestamp":1767609902893,"sessionID":"ses_4723d5c64ffeo3VMtIbToaB7GI","part":{"id":"prt_test","sessionID":"ses_4723d5c64ffeo3VMtIbToaB7GI","messageID":"msg_test","type":"step-start"}}"#;
    const OPENCODE_TEXT_JSON: &str = r#"{"type":"text","timestamp":1767609902907,"sessionID":"ses_4723d5c64ffeo3VMtIbToaB7GI","part":{"id":"prt_text","sessionID":"ses_4723d5c64ffeo3VMtIbToaB7GI","messageID":"msg_text","type":"text","text":"hello"}}"#;
    const OPENCODE_TOOL_USE_JSON: &str = r#"{"type":"tool_use","timestamp":1767609910770,"sessionID":"ses_4723d3e62ffeHCDksgr4fRpmzP","part":{"id":"prt_tool","sessionID":"ses_4723d3e62ffeHCDksgr4fRpmzP","messageID":"msg_tool","type":"tool","callID":"chatcmpl-tool-123","tool":"write","state":{"status":"completed","input":{"content":"hello world","filePath":"/tmp/test.txt"},"output":""}}}"#;
    const OPENCODE_STEP_FINISH_JSON: &str = r#"{"type":"step_finish","timestamp":1767609902907,"sessionID":"ses_4723d5c64ffeo3VMtIbToaB7GI","part":{"id":"prt_finish","sessionID":"ses_4723d5c64ffeo3VMtIbToaB7GI","messageID":"msg_finish","type":"step-finish","reason":"stop","cost":0,"tokens":{"input":10176,"output":2,"reasoning":0}}}"#;

    #[test]
    fn test_parse_opencode_step_start() {
        let events = AgentEvent::parse_opencode(OPENCODE_STEP_START_JSON).unwrap();
        assert_eq!(events.len(), 2);
        // First event should be SessionStarted with UUID
        match &events[0] {
            AgentEvent::SessionStarted { session_id } => {
                // UUID should be deterministic from the session string
                let expected = opencode_session_string_to_uuid("ses_4723d5c64ffeo3VMtIbToaB7GI");
                assert_eq!(*session_id, expected);
            }
            _ => panic!("Expected SessionStarted, got {:?}", events[0]),
        }
        // Second event should be BackgroundEvent with original session string
        match &events[1] {
            AgentEvent::BackgroundEvent { event, extra } => {
                assert_eq!(event.as_deref(), Some("opencode_session"));
                assert_eq!(
                    extra.get("session_string").and_then(|v| v.as_str()),
                    Some("ses_4723d5c64ffeo3VMtIbToaB7GI")
                );
            }
            _ => panic!("Expected BackgroundEvent, got {:?}", events[1]),
        }
    }

    #[test]
    fn test_parse_opencode_text() {
        let events = AgentEvent::parse_opencode(OPENCODE_TEXT_JSON).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.item_type, "agent_message");
                assert_eq!(item.text, Some("hello".to_string()));
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_opencode_tool_use() {
        let events = AgentEvent::parse_opencode(OPENCODE_TOOL_USE_JSON).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.item_type, "tool_call");
                assert_eq!(item.name.as_deref(), Some("write"));
                assert_eq!(item.call_id.as_deref(), Some("chatcmpl-tool-123"));
                assert!(item.arguments.is_some());
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_opencode_step_finish() {
        let events = AgentEvent::parse_opencode(OPENCODE_STEP_FINISH_JSON).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::TurnCompleted { usage } => {
                let usage = usage.as_ref().unwrap();
                assert_eq!(usage.input_tokens, Some(10176));
                assert_eq!(usage.output_tokens, Some(2));
            }
            _ => panic!("Expected TurnCompleted event"),
        }
    }

    #[test]
    fn test_opencode_session_id_extraction() {
        let events = AgentEvent::parse_opencode(OPENCODE_STEP_START_JSON).unwrap();
        // Standard session_id() on SessionStarted event
        let uuid_from_session = events[0].session_id().unwrap();
        let expected_uuid = opencode_session_string_to_uuid("ses_4723d5c64ffeo3VMtIbToaB7GI");
        assert_eq!(uuid_from_session, expected_uuid);

        // opencode_session_id() on BackgroundEvent
        let (uuid, session_str) = events[1].opencode_session_id().unwrap();
        assert_eq!(session_str, "ses_4723d5c64ffeo3VMtIbToaB7GI");
        assert_eq!(uuid, expected_uuid);
    }

    // Gemini event parsing tests
    const GEMINI_INIT_JSON: &str = r#"{"type":"init","timestamp":"2026-01-07T02:18:58.980Z","session_id":"219b0367-780a-4ea0-8ebb-875d740e8fe2","model":"auto-gemini-3"}"#;
    const GEMINI_MESSAGE_JSON: &str = r#"{"type":"message","timestamp":"2026-01-07T02:19:02.137Z","role":"assistant","content":"I will execute the command.","delta":true}"#;
    const GEMINI_TOOL_USE_JSON: &str = r#"{"type":"tool_use","timestamp":"2026-01-07T02:19:02.239Z","tool_name":"run_shell_command","tool_id":"call_123","parameters":{"command":"echo hello"}}"#;
    const GEMINI_TOOL_RESULT_JSON: &str = r#"{"type":"tool_result","timestamp":"2026-01-07T02:19:02.263Z","tool_id":"call_123","status":"success","output":"hello"}"#;
    const GEMINI_RESULT_JSON: &str = r#"{"type":"result","timestamp":"2026-01-07T02:21:30.824Z","status":"success","stats":{"input_tokens":100,"output_tokens":50,"cached":20}}"#;

    #[test]
    fn test_parse_gemini_init() {
        let events = AgentEvent::parse_gemini(GEMINI_INIT_JSON).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::SessionStarted { session_id } => {
                assert_eq!(
                    session_id.to_string(),
                    "219b0367-780a-4ea0-8ebb-875d740e8fe2"
                );
            }
            _ => panic!("Expected SessionStarted event"),
        }
    }

    #[test]
    fn test_parse_gemini_message() {
        let events = AgentEvent::parse_gemini(GEMINI_MESSAGE_JSON).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.item_type, "agent_message");
                assert_eq!(item.text.as_deref(), Some("I will execute the command."));
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_gemini_tool_use() {
        let events = AgentEvent::parse_gemini(GEMINI_TOOL_USE_JSON).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.item_type, "tool_call");
                assert_eq!(item.name.as_deref(), Some("run_shell_command"));
                assert_eq!(item.call_id.as_deref(), Some("call_123"));
                assert!(item.arguments.is_some());
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_gemini_tool_result() {
        let events = AgentEvent::parse_gemini(GEMINI_TOOL_RESULT_JSON).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::ItemCompleted { item } => {
                assert_eq!(item.item_type, "tool_output");
                assert_eq!(item.call_id.as_deref(), Some("call_123"));
                assert_eq!(item.output.as_deref(), Some("hello"));
            }
            _ => panic!("Expected ItemCompleted event"),
        }
    }

    #[test]
    fn test_parse_gemini_result() {
        let events = AgentEvent::parse_gemini(GEMINI_RESULT_JSON).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::TurnCompleted { usage } => {
                let usage = usage.as_ref().unwrap();
                assert_eq!(usage.input_tokens, Some(100));
                assert_eq!(usage.output_tokens, Some(50));
                assert_eq!(usage.cached_input_tokens, Some(20));
            }
            _ => panic!("Expected TurnCompleted event"),
        }
    }
}
