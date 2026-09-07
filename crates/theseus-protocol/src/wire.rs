//! Outward Thread / Turn / Item shapes aligned with openai/codex app-server.
//!
//! This is a *subset*: enough for `thread/start` + `turn/start` and the event
//! stream. Field names are camelCase to match Codex JSON-RPC.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Recent-list row. Not a second source of truth — filled from jsonl mtime + a light read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub path: String,
    pub updated_at: i64,
    #[serde(default)]
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
    #[serde(default)]
    pub preview: String,
    pub model_provider: String,
    /// Unix epoch seconds (Codex `createdAt` convention).
    pub created_at: i64,
    pub updated_at: i64,
    pub status: ThreadStatus,
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ThreadStatus {
    Idle,
    Active {
        #[serde(default, rename = "activeFlags")]
        active_flags: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: String,
    pub status: TurnStatus,
    #[serde(default)]
    pub items: Vec<ThreadItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    InProgress,
    Completed,
    Interrupted,
    Failed,
}

/// Codex-shaped item union used in `item/started` / `item/completed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ThreadItem {
    #[serde(rename_all = "camelCase")]
    UserMessage { id: String, content: Vec<UserInput> },
    #[serde(rename_all = "camelCase")]
    AgentMessage { id: String, text: String },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        id: String,
        name: String,
        arguments: String,
        status: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolResult {
        id: String,
        call_id: String,
        output: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UserInput {
    Text { text: String },
}

impl UserInput {
    pub fn as_text(&self) -> &str {
        match self {
            Self::Text { text } => text,
        }
    }
}

pub fn join_user_input(input: &[UserInput]) -> String {
    input
        .iter()
        .map(UserInput::as_text)
        .collect::<Vec<_>>()
        .join("\n")
}
