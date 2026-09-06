use std::collections::HashMap;

use pi_protocol::{DerivedMessage, EventData, SessionEvent, ToolCallRef};

/// Project model-visible messages from appended events only.
///
/// Single source of truth (π核):
/// - `user/message` → user
/// - `assistant/message` → assistant **text only** (no `tool_calls` on the event)
/// - `tool/call` → the only model-visible tool-call source; merged onto the
///   assistant of the **same** `(turn, step)`, or a new empty-content assistant
/// - `tool/result` → tool
/// - boundaries / `thread/meta` / `assistant/chunk` → nothing
pub fn derive_messages(events: &[SessionEvent]) -> Vec<DerivedMessage> {
    let mut out = Vec::new();
    // (turn, step) → index of that step's assistant in `out`.
    let mut assistant_at: HashMap<(u32, u32), usize> = HashMap::new();

    for event in events {
        match &event.data {
            EventData::UserMessage(m) => {
                out.push(DerivedMessage::User {
                    content: m.content.clone(),
                });
            }
            EventData::AssistantMessage(m) => {
                let text = m.message.content.as_str();
                let key = (m.turn, m.step);
                if let Some(&idx) = assistant_at.get(&key) {
                    if let DerivedMessage::Assistant { content, .. } = &mut out[idx] {
                        if content.is_empty() && !text.is_empty() {
                            *content = text.to_string();
                        }
                    }
                } else if !text.is_empty() {
                    assistant_at.insert(key, out.len());
                    out.push(DerivedMessage::Assistant {
                        content: text.to_string(),
                        tool_calls: vec![],
                    });
                }
            }
            EventData::ToolCall(c) => {
                let call = ToolCallRef {
                    call_id: c.call_id.clone(),
                    name: c.name.clone(),
                    arguments: c.arguments.clone(),
                };
                let key = (c.turn, c.step);
                if let Some(&idx) = assistant_at.get(&key) {
                    if let DerivedMessage::Assistant { tool_calls, .. } = &mut out[idx] {
                        if !tool_calls
                            .iter()
                            .any(|existing| existing.call_id == call.call_id)
                        {
                            tool_calls.push(call);
                        }
                    }
                } else {
                    assistant_at.insert(key, out.len());
                    out.push(DerivedMessage::Assistant {
                        content: String::new(),
                        tool_calls: vec![call],
                    });
                }
            }
            EventData::ToolResult(r) => {
                out.push(DerivedMessage::Tool {
                    tool_call_id: r.call_id.clone(),
                    content: r.content.clone(),
                    is_error: r.is_error,
                });
            }
            EventData::ThreadMeta(_)
            | EventData::TurnStart(_)
            | EventData::TurnEnd(_)
            | EventData::StepStart(_)
            | EventData::StepEnd(_)
            | EventData::AssistantChunk(_) => {}
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::{
        AssistantMessageBody, AssistantMessageEvent, ToolCallEvent, ToolResultEvent,
        UserMessageEvent,
    };

    fn ev(seq: u64, data: EventData) -> SessionEvent {
        SessionEvent {
            seq,
            time: seq as i64,
            data,
        }
    }

    fn assistant(turn: u32, step: u32, text: &str) -> EventData {
        EventData::AssistantMessage(AssistantMessageEvent {
            turn,
            step,
            message: AssistantMessageBody::text(format!("a_{turn}_{step}"), text),
            usage: None,
            interrupted: None,
        })
    }

    fn tool_call(turn: u32, step: u32, call_id: &str) -> EventData {
        EventData::ToolCall(ToolCallEvent {
            turn,
            step,
            call_id: call_id.into(),
            name: "echo".into(),
            arguments: "{}".into(),
        })
    }

    #[test]
    fn skips_empty_assistant_and_boundaries() {
        let events = vec![
            ev(0, EventData::TurnStart(pi_protocol::TurnStart { turn: 1 })),
            ev(
                1,
                EventData::UserMessage(UserMessageEvent {
                    turn: 1,
                    id: "u".into(),
                    content: "q".into(),
                    source: None,
                }),
            ),
            ev(2, assistant(1, 1, "")),
        ];
        assert_eq!(
            derive_messages(&events),
            vec![DerivedMessage::User {
                content: "q".into()
            }]
        );
    }

    #[test]
    fn ignores_assistant_message_tool_calls_field() {
        let planted = serde_json::json!({
            "seq": 0,
            "time": 0,
            "type": "assistant/message",
            "data": {
                "turn": 1,
                "step": 1,
                "message": {
                    "id": "a_1_1",
                    "content": "hi",
                    "tool_calls": [{
                        "callId": "planted",
                        "name": "echo",
                        "arguments": "{}"
                    }]
                }
            }
        });
        let ev: SessionEvent = serde_json::from_value(planted).unwrap();
        let written = serde_json::to_value(&ev).unwrap();
        assert!(
            written["data"]["message"].get("tool_calls").is_none(),
            "re-serialize must not emit tool_calls: {written}"
        );
        match &derive_messages(&[ev])[0] {
            DerivedMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "hi");
                assert!(
                    tool_calls.is_empty(),
                    "derive must not read message.tool_calls"
                );
            }
            other => panic!("expected assistant, got {other:?}"),
        }
    }

    #[test]
    fn same_step_assistant_then_tool_call_merges_once() {
        let events = vec![
            ev(0, assistant(1, 1, "calling")),
            ev(1, tool_call(1, 1, "c1")),
            ev(
                2,
                EventData::ToolResult(ToolResultEvent {
                    turn: 1,
                    step: 1,
                    call_id: "c1".into(),
                    content: "ok".into(),
                    is_error: false,
                }),
            ),
        ];
        let msgs = derive_messages(&events);
        assert_eq!(msgs.len(), 2);
        match &msgs[0] {
            DerivedMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "calling");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].call_id, "c1");
            }
            other => panic!("expected one merged assistant, got {other:?}"),
        }
        match &msgs[1] {
            DerivedMessage::Tool { tool_call_id, .. } => assert_eq!(tool_call_id, "c1"),
            other => panic!("expected tool, got {other:?}"),
        }
    }

    #[test]
    fn same_step_tool_call_then_assistant_does_not_duplicate_calls() {
        let events = vec![
            ev(0, tool_call(1, 1, "c1")),
            ev(1, assistant(1, 1, "calling")),
        ];
        let msgs = derive_messages(&events);
        assert_eq!(msgs.len(), 1, "must be one assistant, not two");
        match &msgs[0] {
            DerivedMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "calling");
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].call_id, "c1");
            }
            other => panic!("expected assistant, got {other:?}"),
        }
    }

    #[test]
    fn cross_step_tool_call_does_not_hang_on_previous_assistant() {
        let events = vec![
            ev(0, assistant(1, 1, "step one")),
            ev(
                1,
                EventData::StepEnd(pi_protocol::StepEnd { turn: 1, step: 1 }),
            ),
            ev(
                2,
                EventData::StepStart(pi_protocol::StepStart { turn: 1, step: 2 }),
            ),
            ev(3, tool_call(1, 2, "c2")),
        ];
        let msgs = derive_messages(&events);
        assert_eq!(msgs.len(), 2);
        match &msgs[0] {
            DerivedMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "step one");
                assert!(
                    tool_calls.is_empty(),
                    "step-2 tool/call must not attach to step-1 assistant"
                );
            }
            other => panic!("expected step-1 assistant, got {other:?}"),
        }
        match &msgs[1] {
            DerivedMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert!(content.is_empty());
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].call_id, "c2");
            }
            other => panic!("expected new step-2 assistant, got {other:?}"),
        }
    }
}
