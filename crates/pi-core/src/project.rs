use pi_protocol::{EventData, SessionEvent, ThreadItem, UserInput};

/// Codex-shaped Item projection of a single SessionEvent.
///
/// Same function the app-server must use for `item/started` / `item/completed`.
/// Boundary / meta / chunk events produce no item.
pub fn project_item(event: &SessionEvent) -> Option<ThreadItem> {
    match &event.data {
        EventData::UserMessage(m) => Some(ThreadItem::UserMessage {
            id: m.id.clone(),
            content: vec![UserInput::Text {
                text: m.content.clone(),
            }],
        }),
        EventData::AssistantMessage(m) => Some(ThreadItem::AgentMessage {
            id: m.message.id.clone(),
            text: m.message.content.clone(),
        }),
        EventData::ToolCall(c) => Some(ThreadItem::ToolCall {
            id: c.call_id.clone(),
            name: c.name.clone(),
            arguments: c.arguments.clone(),
            status: "inProgress".into(),
        }),
        EventData::ToolResult(r) => Some(ThreadItem::ToolResult {
            id: format!("result_{}", r.call_id),
            call_id: r.call_id.clone(),
            output: r.content.clone(),
            is_error: r.is_error,
        }),
        EventData::ThreadMeta(_)
        | EventData::TurnStart(_)
        | EventData::TurnEnd(_)
        | EventData::StepStart(_)
        | EventData::StepEnd(_)
        | EventData::AssistantChunk(_) => None,
    }
}

/// Project every item-producing event in order.
pub fn project_items(events: &[SessionEvent]) -> Vec<ThreadItem> {
    events.iter().filter_map(project_item).collect()
}

/// Items belonging to one turn (by the event's `turn` field).
pub fn project_items_for_turn(events: &[SessionEvent], turn: u32) -> Vec<ThreadItem> {
    events
        .iter()
        .filter(|event| event.data.turn() == Some(turn))
        .filter_map(project_item)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_protocol::{
        AssistantMessageBody, AssistantMessageEvent, ToolCallEvent, UserMessageEvent,
    };

    fn ev(seq: u64, data: EventData) -> SessionEvent {
        SessionEvent {
            seq,
            time: seq as i64,
            data,
        }
    }

    #[test]
    fn projects_surface_events_and_skips_boundaries() {
        let events = vec![
            ev(0, EventData::TurnStart(pi_protocol::TurnStart { turn: 1 })),
            ev(
                1,
                EventData::UserMessage(UserMessageEvent {
                    turn: 1,
                    id: "item_u".into(),
                    content: "hello".into(),
                    source: None,
                }),
            ),
            ev(
                2,
                EventData::AssistantMessage(AssistantMessageEvent {
                    turn: 1,
                    step: 1,
                    message: AssistantMessageBody::text("item_a", "hi"),
                    usage: None,
                    interrupted: None,
                }),
            ),
            ev(
                3,
                EventData::ToolCall(ToolCallEvent {
                    turn: 1,
                    step: 1,
                    call_id: "c1".into(),
                    name: "echo".into(),
                    arguments: "{}".into(),
                }),
            ),
        ];
        let items = project_items(&events);
        assert_eq!(
            items,
            vec![
                ThreadItem::UserMessage {
                    id: "item_u".into(),
                    content: vec![UserInput::Text {
                        text: "hello".into()
                    }],
                },
                ThreadItem::AgentMessage {
                    id: "item_a".into(),
                    text: "hi".into(),
                },
                ThreadItem::ToolCall {
                    id: "c1".into(),
                    name: "echo".into(),
                    arguments: "{}".into(),
                    status: "inProgress".into(),
                },
            ]
        );
        assert_eq!(project_items_for_turn(&events, 2), vec![]);
    }
}
