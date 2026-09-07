use serde::Serialize;
use serde_json::Value;
use theseus_protocol::DerivedMessage;

/// One OpenAI-compatible chat message (request body only).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<OpenAiToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub function: OpenAiFunctionCall,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiFunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiTool {
    #[serde(rename = "type")]
    pub type_: String,
    pub function: OpenAiToolFunction,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenAiToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl OpenAiTool {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
    ) -> Self {
        Self {
            type_: "function".into(),
            function: OpenAiToolFunction {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// Map `derive_messages` output to OpenAI `messages`.
///
/// Assistant tool calls in the derived list already come only from `tool/call` events.
pub fn messages_from_derived(messages: &[DerivedMessage]) -> Vec<OpenAiChatMessage> {
    messages
        .iter()
        .map(|m| match m {
            DerivedMessage::User { content } => OpenAiChatMessage {
                role: "user".into(),
                content: Some(content.clone()),
                tool_calls: vec![],
                tool_call_id: None,
            },
            DerivedMessage::Assistant {
                content,
                tool_calls,
            } => OpenAiChatMessage {
                role: "assistant".into(),
                content: if content.is_empty() && !tool_calls.is_empty() {
                    None
                } else {
                    Some(content.clone())
                },
                tool_calls: tool_calls
                    .iter()
                    .map(|c| OpenAiToolCall {
                        id: c.call_id.clone(),
                        type_: "function".into(),
                        function: OpenAiFunctionCall {
                            name: c.name.clone(),
                            arguments: c.arguments.clone(),
                        },
                    })
                    .collect(),
                tool_call_id: None,
            },
            DerivedMessage::Tool {
                tool_call_id,
                content,
                ..
            } => OpenAiChatMessage {
                role: "tool".into(),
                content: Some(content.clone()),
                tool_calls: vec![],
                tool_call_id: Some(tool_call_id.clone()),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use theseus_protocol::ToolCallRef;

    #[test]
    fn maps_user_assistant_tool() {
        let msgs = messages_from_derived(&[
            DerivedMessage::User {
                content: "hi".into(),
            },
            DerivedMessage::Assistant {
                content: "call".into(),
                tool_calls: vec![ToolCallRef {
                    call_id: "c1".into(),
                    name: "echo".into(),
                    arguments: "{}".into(),
                }],
            },
            DerivedMessage::Tool {
                tool_call_id: "c1".into(),
                content: "ok".into(),
                is_error: false,
            },
        ]);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].tool_calls[0].id, "c1");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("c1"));
    }
}
