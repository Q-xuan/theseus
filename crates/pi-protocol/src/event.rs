use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::HISTORY_EVENT_TYPES;

/// One immutable entry in a session log.
///
/// Wire shape matches deepseek-harness: `{ type, seq, time, data }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionEvent {
    /// Monotonic sequence number. Equals the log index (`seq == events.len()` on append).
    pub seq: u64,
    /// Unix epoch milliseconds.
    pub time: i64,
    #[serde(flatten)]
    pub data: EventData,
}

/// Discriminated payload. `type` + `data` pair so `switch (event.type)` is lossless.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "data")]
pub enum EventData {
    #[serde(rename = "thread/meta")]
    ThreadMeta(ThreadMeta),
    #[serde(rename = "turn/start")]
    TurnStart(TurnStart),
    #[serde(rename = "turn/end")]
    TurnEnd(TurnEnd),
    #[serde(rename = "step/start")]
    StepStart(StepStart),
    #[serde(rename = "step/end")]
    StepEnd(StepEnd),
    #[serde(rename = "user/message")]
    UserMessage(UserMessageEvent),
    #[serde(rename = "assistant/message")]
    AssistantMessage(AssistantMessageEvent),
    #[serde(rename = "tool/call")]
    ToolCall(ToolCallEvent),
    #[serde(rename = "tool/result")]
    ToolResult(ToolResultEvent),
    /// Streaming UX only. Rejected by the session log; never derived into messages.
    #[serde(rename = "assistant/chunk")]
    AssistantChunk(AssistantChunk),
}

impl EventData {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::ThreadMeta(_) => "thread/meta",
            Self::TurnStart(_) => "turn/start",
            Self::TurnEnd(_) => "turn/end",
            Self::StepStart(_) => "step/start",
            Self::StepEnd(_) => "step/end",
            Self::UserMessage(_) => "user/message",
            Self::AssistantMessage(_) => "assistant/message",
            Self::ToolCall(_) => "tool/call",
            Self::ToolResult(_) => "tool/result",
            Self::AssistantChunk(_) => "assistant/chunk",
        }
    }

    /// The frozen nine-kind history set. Chunks are excluded.
    pub fn is_history(&self) -> bool {
        HISTORY_EVENT_TYPES.contains(&self.type_name())
    }

    pub fn turn(&self) -> Option<u32> {
        match self {
            Self::ThreadMeta(_) => None,
            Self::TurnStart(t) => Some(t.turn),
            Self::TurnEnd(t) => Some(t.turn),
            Self::StepStart(s) => Some(s.turn),
            Self::StepEnd(s) => Some(s.turn),
            Self::UserMessage(m) => Some(m.turn),
            Self::AssistantMessage(m) => Some(m.turn),
            Self::ToolCall(c) => Some(c.turn),
            Self::ToolResult(r) => Some(r.turn),
            Self::AssistantChunk(c) => Some(c.turn),
        }
    }

    pub fn step(&self) -> Option<u32> {
        match self {
            Self::StepStart(s) => Some(s.step),
            Self::StepEnd(s) => Some(s.step),
            Self::AssistantMessage(m) => Some(m.step),
            Self::ToolCall(c) => Some(c.step),
            Self::ToolResult(r) => Some(r.step),
            Self::AssistantChunk(c) => Some(c.step),
            Self::ThreadMeta(_) | Self::TurnStart(_) | Self::TurnEnd(_) | Self::UserMessage(_) => {
                None
            }
        }
    }
}

impl SessionEvent {
    pub fn type_name(&self) -> &'static str {
        self.data.type_name()
    }

    pub fn is_history(&self) -> bool {
        self.data.is_history()
    }
}

/// First event of a thread. Records identity; does not project into model messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMeta {
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Unix epoch milliseconds.
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TurnStart {
    pub turn: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TurnEnd {
    pub turn: u32,
    pub reason: TurnEndReason,
}

/// Why a turn closed. Smaller than deepseek-harness; `kind` is kebab-case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum TurnEndReason {
    Completed,
    Aborted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Error {
        message: String,
    },
    Interrupted,
    #[serde(rename = "max-tokens")]
    MaxTokens,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StepStart {
    pub turn: u32,
    pub step: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StepEnd {
    pub turn: u32,
    pub step: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserMessageEvent {
    pub turn: u32,
    pub id: String,
    pub content: String,
    /// `"user"` (human prompt) or `"inject"` (synthetic context). Default: user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessageEvent {
    pub turn: u32,
    pub step: u32,
    pub message: AssistantMessageBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessageBody {
    pub id: String,
    /// Model-visible assistant text. Tool calls are never stored here;
    /// the only model-visible source is an appended `tool/call` event.
    pub content: String,
}

impl AssistantMessageBody {
    pub fn text(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallRef {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEvent {
    pub turn: u32,
    pub step: u32,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultEvent {
    pub turn: u32,
    pub step: u32,
    pub call_id: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

/// Token-level streaming fragment. Protocol type only — not a history event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AssistantChunk {
    pub turn: u32,
    pub step: u32,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HISTORY_EVENT_TYPES;

    fn sample_events() -> Vec<SessionEvent> {
        vec![
            SessionEvent {
                seq: 0,
                time: 1,
                data: EventData::ThreadMeta(ThreadMeta {
                    thread_id: "thr_1".into(),
                    model: Some("stub".into()),
                    cwd: Some("/tmp".into()),
                    title: None,
                    created_at: 1,
                    parent_thread_id: None,
                }),
            },
            SessionEvent {
                seq: 1,
                time: 2,
                data: EventData::TurnStart(TurnStart { turn: 1 }),
            },
            SessionEvent {
                seq: 2,
                time: 3,
                data: EventData::UserMessage(UserMessageEvent {
                    turn: 1,
                    id: "item_u".into(),
                    content: "hello".into(),
                    source: Some("user".into()),
                }),
            },
            SessionEvent {
                seq: 3,
                time: 4,
                data: EventData::StepStart(StepStart { turn: 1, step: 1 }),
            },
            SessionEvent {
                seq: 4,
                time: 5,
                data: EventData::AssistantMessage(AssistantMessageEvent {
                    turn: 1,
                    step: 1,
                    message: AssistantMessageBody::text("item_a", "hi"),
                    usage: Some(TokenUsage {
                        input_tokens: Some(3),
                        output_tokens: Some(1),
                    }),
                    interrupted: None,
                }),
            },
            SessionEvent {
                seq: 5,
                time: 6,
                data: EventData::ToolCall(ToolCallEvent {
                    turn: 1,
                    step: 1,
                    call_id: "call_1".into(),
                    name: "echo".into(),
                    arguments: "{\"x\":1}".into(),
                }),
            },
            SessionEvent {
                seq: 6,
                time: 7,
                data: EventData::ToolResult(ToolResultEvent {
                    turn: 1,
                    step: 1,
                    call_id: "call_1".into(),
                    content: "1".into(),
                    is_error: false,
                }),
            },
            SessionEvent {
                seq: 7,
                time: 8,
                data: EventData::StepEnd(StepEnd { turn: 1, step: 1 }),
            },
            SessionEvent {
                seq: 8,
                time: 9,
                data: EventData::TurnEnd(TurnEnd {
                    turn: 1,
                    reason: TurnEndReason::Completed,
                }),
            },
            SessionEvent {
                seq: 9,
                time: 10,
                data: EventData::AssistantChunk(AssistantChunk {
                    turn: 1,
                    step: 1,
                    text: "h".into(),
                }),
            },
        ]
    }

    #[test]
    fn json_round_trip_all_kinds() {
        for event in sample_events() {
            let json = serde_json::to_value(&event).expect("serialize");
            assert_eq!(json["type"], event.type_name());
            assert_eq!(json["seq"], event.seq);
            assert!(json.get("data").is_some());
            let back: SessionEvent = serde_json::from_value(json).expect("deserialize");
            assert_eq!(back, event);
        }
    }

    #[test]
    fn history_set_is_exactly_nine() {
        assert_eq!(HISTORY_EVENT_TYPES.len(), 9);
        let mut history: Vec<_> = sample_events()
            .into_iter()
            .filter(|e| e.is_history())
            .map(|e| e.type_name())
            .collect();
        history.sort_unstable();
        let mut expected = HISTORY_EVENT_TYPES.to_vec();
        expected.sort_unstable();
        assert_eq!(history, expected);
    }

    #[test]
    fn turn_end_reason_kind_is_kebab_case() {
        let reason = TurnEndReason::MaxTokens;
        let v = serde_json::to_value(reason).unwrap();
        assert_eq!(v["kind"], "max-tokens");
    }

    #[test]
    fn assistant_message_json_omits_tool_calls() {
        let event = SessionEvent {
            seq: 0,
            time: 1,
            data: EventData::AssistantMessage(AssistantMessageEvent {
                turn: 1,
                step: 1,
                message: AssistantMessageBody::text("item_a", "hi"),
                usage: None,
                interrupted: None,
            }),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert!(
            json["data"].get("tool_calls").is_none(),
            "assistant/message must not write tool_calls: {json}"
        );
        assert!(
            json["data"]["message"].get("tool_calls").is_none(),
            "assistant/message must not write tool_calls: {json}"
        );

        let legacy = serde_json::json!({
            "seq": 0,
            "time": 1,
            "type": "assistant/message",
            "data": {
                "turn": 1,
                "step": 1,
                "message": {
                    "id": "item_a",
                    "content": "hi",
                    "tool_calls": [{
                        "callId": "planted",
                        "name": "echo",
                        "arguments": "{}"
                    }]
                }
            }
        });
        let back: SessionEvent = serde_json::from_value(legacy).expect("legacy tool_calls ignored");
        match &back.data {
            EventData::AssistantMessage(m) => {
                assert_eq!(m.message.content, "hi");
                let again = serde_json::to_value(&back).unwrap();
                assert!(again["data"]["message"].get("tool_calls").is_none());
            }
            other => panic!("expected assistant/message, got {other:?}"),
        }
    }
}
