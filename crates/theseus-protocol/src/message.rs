use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::ToolCallRef;

/// Model-visible message projected from the session log (`derive_messages`).
///
/// Assistant `tool_calls` are filled only from `tool/call` history events,
/// never from `assistant/message` (that event has no `tool_calls` field).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum DerivedMessage {
    #[serde(rename = "user")]
    User { content: String },
    #[serde(rename = "assistant")]
    Assistant {
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCallRef>,
    },
    #[serde(rename = "tool")]
    Tool {
        tool_call_id: String,
        content: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}
