//! One event map: `pi --mode rpc` events → Thread / Turn / Item notifications.
//!
//! The desktop shell projects these notifications. It does not run
//! `derive_messages` or the tool loop.

use serde_json::{json, Value};

fn notify(method: &str, params: Value) -> Value {
    json!({ "method": method, "params": params })
}

fn item_started(item: Value) -> Value {
    notify("item/started", json!({ "item": item }))
}

fn item_completed(item: Value) -> Value {
    notify("item/completed", json!({ "item": item }))
}

fn user_text(message: &Value) -> String {
    let content = &message["content"];
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("image") {
                    return None;
                }
                block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

fn assistant_text(message: &Value) -> String {
    let content = &message["content"];
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("thinking") {
                    return None;
                }
                block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<Vec<_>>()
            .join("");
    }
    String::new()
}

fn message_id(message: &Value, fallback: &str) -> String {
    message
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn message_role(message: &Value) -> &str {
    message.get("role").and_then(|r| r.as_str()).unwrap_or("")
}

fn tool_args_value(raw: &Value) -> Value {
    if raw.is_null() {
        json!({})
    } else if raw.is_string() || raw.is_object() {
        raw.clone()
    } else {
        json!(raw.to_string())
    }
}

fn tool_output(result: &Value) -> String {
    if let Some(s) = result.as_str() {
        return s.to_string();
    }
    if let Some(content) = result.get("content") {
        if let Some(s) = content.as_str() {
            return s.to_string();
        }
        if let Some(arr) = content.as_array() {
            return arr
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("");
        }
    }
    if result.is_object() || result.is_array() {
        return result.to_string();
    }
    String::new()
}

fn user_item(id: &str, text: &str) -> Value {
    json!({
        "type": "userMessage",
        "id": id,
        "content": [{ "type": "text", "text": text }],
    })
}

fn agent_item(id: &str, text: &str) -> Value {
    json!({
        "type": "agentMessage",
        "id": id,
        "text": text,
    })
}

fn tool_call_item(id: &str, name: &str, arguments: Value) -> Value {
    json!({
        "type": "toolCall",
        "id": id,
        "name": name,
        "arguments": arguments,
        "status": "inProgress",
    })
}

fn tool_result_item(call_id: &str, output: &str, is_error: bool) -> Value {
    json!({
        "type": "toolResult",
        "id": format!("result_{call_id}"),
        "callId": call_id,
        "output": output,
        "isError": is_error,
    })
}

/// Incremental map from a live `pi` event stream.
#[derive(Debug, Default)]
pub struct EventMap {
    pub turn_count: u32,
    pub streaming: bool,
    pub aborted: bool,
    next_item: u32,
    current_assistant_id: Option<String>,
    assistant_text: String,
}

impl EventMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_abort(&mut self) {
        self.aborted = true;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    fn alloc_id(&mut self, prefix: &str) -> String {
        self.next_item += 1;
        format!("{prefix}_{}", self.next_item)
    }

    /// Ingest one pi event. Returns JSON-RPC-shaped notifications
    /// (`{method, params}`) in Thread / Turn / Item vocabulary.
    pub fn ingest(&mut self, event: &Value) -> Vec<Value> {
        let typ = event.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match typ {
            "agent_start" => {
                self.streaming = true;
                self.aborted = false;
                vec![notify(
                    "agent_start",
                    json!({ "isStreaming": true }),
                )]
            }
            "agent_settled" => {
                self.streaming = false;
                let status = if self.aborted {
                    "interrupted"
                } else {
                    "completed"
                };
                vec![
                    notify("agent_settled", json!({ "isStreaming": false })),
                    notify(
                        "turn/completed",
                        json!({ "turn": { "status": status } }),
                    ),
                ]
            }
            "turn_start" => {
                self.turn_count += 1;
                vec![notify(
                    "turn/started",
                    json!({ "turn": { "id": format!("turn_{}", self.turn_count) } }),
                )]
            }
            "turn_end" => Vec::new(),
            "message_start" => self.on_message_start(event),
            "message_update" => self.on_message_update(event),
            "message_end" => self.on_message_end(event),
            "tool_execution_start" => self.on_tool_start(event),
            "tool_execution_update" => self.on_tool_update(event),
            "tool_execution_end" => self.on_tool_end(event),
            _ => Vec::new(),
        }
    }

    fn on_message_start(&mut self, event: &Value) -> Vec<Value> {
        let message = &event["message"];
        match message_role(message) {
            "user" => {
                let id = message_id(message, &self.alloc_id("user"));
                let text = user_text(message);
                vec![item_started(user_item(&id, &text))]
            }
            "assistant" => {
                let id = message_id(message, &self.alloc_id("agent"));
                self.current_assistant_id = Some(id.clone());
                self.assistant_text.clear();
                vec![item_started(agent_item(&id, ""))]
            }
            "toolResult" => {
                let call_id = message
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                let output = tool_output(message);
                let is_error = message
                    .get("isError")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                vec![item_started(tool_result_item(call_id, &output, is_error))]
            }
            _ => Vec::new(),
        }
    }

    fn on_message_update(&mut self, event: &Value) -> Vec<Value> {
        let ame = &event["assistantMessageEvent"];
        let kind = ame.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match kind {
            "text_delta" => {
                let delta = ame.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                if delta.is_empty() {
                    return Vec::new();
                }
                self.assistant_text.push_str(delta);
                let id = self
                    .current_assistant_id
                    .clone()
                    .unwrap_or_else(|| self.alloc_id("agent"));
                self.current_assistant_id = Some(id.clone());
                vec![notify(
                    "item/agentMessage/delta",
                    json!({ "itemId": id, "delta": delta }),
                )]
            }
            "toolcall_start" => {
                let id = ame
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| self.alloc_id("tool"));
                let name = ame
                    .get("toolName")
                    .or_else(|| ame.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                vec![item_started(tool_call_item(&id, name, json!({})))]
            }
            _ => Vec::new(),
        }
    }

    fn on_message_end(&mut self, event: &Value) -> Vec<Value> {
        let message = &event["message"];
        let mut out = Vec::new();
        match message_role(message) {
            "user" => {
                let id = message_id(message, &self.alloc_id("user"));
                out.push(item_completed(user_item(&id, &user_text(message))));
            }
            "assistant" => {
                let id = message_id(
                    message,
                    self.current_assistant_id
                        .as_deref()
                        .unwrap_or("agent"),
                );
                let text = {
                    let finished = assistant_text(message);
                    if finished.is_empty() {
                        self.assistant_text.clone()
                    } else {
                        finished
                    }
                };
                out.push(item_completed(agent_item(&id, &text)));
                if let Some(arr) = message.get("content").and_then(|c| c.as_array()) {
                    for block in arr {
                        if block.get("type").and_then(|t| t.as_str()) != Some("toolCall") {
                            continue;
                        }
                        let call_id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool");
                        let name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("tool");
                        let args = tool_args_value(&block["arguments"]);
                        out.push(item_completed(tool_call_item(call_id, name, args)));
                    }
                }
                self.current_assistant_id = None;
                self.assistant_text.clear();
            }
            "toolResult" => {
                let call_id = message
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                let output = tool_output(message);
                let is_error = message
                    .get("isError")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                out.push(item_completed(tool_result_item(call_id, &output, is_error)));
            }
            _ => {}
        }
        out
    }

    fn on_tool_start(&mut self, event: &Value) -> Vec<Value> {
        let id = event
            .get("toolCallId")
            .and_then(|v| v.as_str())
            .unwrap_or("tool");
        let name = event
            .get("toolName")
            .and_then(|v| v.as_str())
            .unwrap_or("tool");
        let args = tool_args_value(&event["args"]);
        vec![item_started(tool_call_item(id, name, args))]
    }

    fn on_tool_update(&mut self, event: &Value) -> Vec<Value> {
        let id = event
            .get("toolCallId")
            .and_then(|v| v.as_str())
            .unwrap_or("tool");
        let name = event
            .get("toolName")
            .and_then(|v| v.as_str())
            .unwrap_or("tool");
        let output = tool_output(&event["partialResult"]);
        let _ = name;
        vec![item_completed(tool_result_item(id, &output, false))]
    }

    fn on_tool_end(&mut self, event: &Value) -> Vec<Value> {
        let id = event
            .get("toolCallId")
            .and_then(|v| v.as_str())
            .unwrap_or("tool");
        let output = tool_output(&event["result"]);
        let is_error = event
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        vec![item_completed(tool_result_item(id, &output, is_error))]
    }
}

fn first_user_preview(messages: &[Value]) -> String {
    for message in messages {
        if message_role(message) == "user" {
            let text = user_text(message);
            if !text.is_empty() {
                return text;
            }
        }
    }
    String::new()
}

/// Project `get_messages` / message arrays into a Thread with Turns and Items.
pub fn project_messages(session_id: &str, messages: &[Value]) -> Value {
    let mut turns: Vec<Value> = Vec::new();
    let mut items: Vec<Value> = Vec::new();
    let mut turn_n = 0u32;

    let flush = |turns: &mut Vec<Value>, items: &mut Vec<Value>, turn_n: u32| {
        if items.is_empty() && turn_n == 0 {
            return;
        }
        if turn_n == 0 {
            return;
        }
        turns.push(json!({
            "id": format!("turn_{session_id}_{turn_n}"),
            "status": "completed",
            "items": items.clone(),
        }));
        items.clear();
    };

    for message in messages {
        match message_role(message) {
            "user" => {
                flush(&mut turns, &mut items, turn_n);
                turn_n += 1;
                let id = message_id(message, &format!("user_{turn_n}"));
                items.push(user_item(&id, &user_text(message)));
            }
            "assistant" => {
                if turn_n == 0 {
                    turn_n = 1;
                }
                let id = message_id(message, &format!("agent_{turn_n}"));
                items.push(agent_item(&id, &assistant_text(message)));
                if let Some(arr) = message.get("content").and_then(|c| c.as_array()) {
                    for block in arr {
                        if block.get("type").and_then(|t| t.as_str()) != Some("toolCall") {
                            continue;
                        }
                        let call_id = block.get("id").and_then(|v| v.as_str()).unwrap_or("tool");
                        let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                        items.push(tool_call_item(call_id, name, tool_args_value(&block["arguments"])));
                    }
                }
            }
            "toolResult" => {
                if turn_n == 0 {
                    turn_n = 1;
                }
                let call_id = message
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                let is_error = message
                    .get("isError")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                items.push(tool_result_item(call_id, &tool_output(message), is_error));
            }
            _ => {}
        }
    }
    flush(&mut turns, &mut items, turn_n);

    json!({
        "id": session_id,
        "preview": first_user_preview(messages),
        "turns": turns,
    })
}

/// Project `get_entries` into the same Thread shape.
pub fn project_entries(session_id: &str, entries: &[Value]) -> Value {
    let messages: Vec<Value> = entries
        .iter()
        .filter_map(|entry| {
            if let Some(message) = entry.get("message") {
                return Some(message.clone());
            }
            let role = entry.get("role").and_then(|r| r.as_str())?;
            if matches!(role, "user" | "assistant" | "toolResult") {
                Some(entry.clone())
            } else {
                None
            }
        })
        .collect();
    project_messages(session_id, &messages)
}

/// Attach a projected `thread` object onto a pi command result.
pub fn enrich_result(command: &str, session_id: &str, data: &Value) -> Value {
    let mut out = data.clone();
    if !out.is_object() {
        out = json!({ "data": data });
    }
    match command {
        "get_messages" => {
            let messages = data
                .get("messages")
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_default();
            if let Some(obj) = out.as_object_mut() {
                obj.insert(
                    "thread".into(),
                    project_messages(session_id, &messages),
                );
            }
        }
        "get_entries" => {
            let entries = data
                .get("entries")
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_default();
            if let Some(obj) = out.as_object_mut() {
                obj.insert("thread".into(), project_entries(session_id, &entries));
            }
        }
        _ => {}
    }
    out
}

pub fn pi_commands() -> &'static [&'static str] {
    &[
        "new_session",
        "prompt",
        "abort",
        "clear_queue",
        "get_state",
        "get_messages",
        "get_entries",
        "switch_session",
        "set_session_name",
    ]
}

pub fn is_pi_command(method: &str) -> bool {
    pi_commands().contains(&method)
}

pub fn is_theseus_app_server_method(method: &str) -> bool {
    matches!(
        method,
        "initialize"
            | "shutdown"
            | "thread/start"
            | "thread/resume"
            | "thread/list"
            | "turn/start"
            | "turn/interrupt"
            | "tool/approve"
            | "tool/reject"
            | "session/event"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(v: Value) -> Vec<Value> {
        let mut map = EventMap::new();
        map.ingest(&v)
    }

    #[test]
    fn agent_start_and_settled_drive_streaming() {
        let mut map = EventMap::new();
        let start = map.ingest(&json!({"type":"agent_start"}));
        assert_eq!(start[0]["method"], "agent_start");
        assert_eq!(start[0]["params"]["isStreaming"], true);
        map.mark_abort();
        let end = map.ingest(&json!({"type":"agent_settled"}));
        assert_eq!(end[0]["method"], "agent_settled");
        assert_eq!(end[0]["params"]["isStreaming"], false);
        assert_eq!(end[1]["method"], "turn/completed");
        assert_eq!(end[1]["params"]["turn"]["status"], "interrupted");
        assert!(!map.streaming);
    }

    #[test]
    fn message_update_becomes_agent_delta() {
        let mut map = EventMap::new();
        let started = map.ingest(&json!({
            "type":"message_start",
            "message":{"role":"assistant","id":"a1","content":[]}
        }));
        assert_eq!(started[0]["params"]["item"]["type"], "agentMessage");
        let delta = map.ingest(&json!({
            "type":"message_update",
            "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"hi"}
        }));
        assert_eq!(delta[0]["method"], "item/agentMessage/delta");
        assert_eq!(delta[0]["params"]["itemId"], "a1");
        assert_eq!(delta[0]["params"]["delta"], "hi");
    }

    #[test]
    fn tool_execution_becomes_one_line_items() {
        let mut map = EventMap::new();
        let start = map.ingest(&json!({
            "type":"tool_execution_start",
            "toolCallId":"c1",
            "toolName":"bash",
            "args":{"command":"ls"}
        }));
        assert_eq!(start[0]["params"]["item"]["type"], "toolCall");
        assert_eq!(start[0]["params"]["item"]["name"], "bash");
        let end = map.ingest(&json!({
            "type":"tool_execution_end",
            "toolCallId":"c1",
            "toolName":"bash",
            "result":{"content":[{"type":"text","text":"ok"}]},
            "isError":false
        }));
        assert_eq!(end[0]["params"]["item"]["type"], "toolResult");
        assert_eq!(end[0]["params"]["item"]["output"], "ok");
    }

    #[test]
    fn project_messages_groups_turns() {
        let thread = project_messages(
            "s1",
            &[
                json!({"role":"user","content":"ping","id":"u1"}),
                json!({"role":"assistant","content":[{"type":"text","text":"pong"}],"id":"a1"}),
            ],
        );
        assert_eq!(thread["id"], "s1");
        assert_eq!(thread["preview"], "ping");
        assert_eq!(thread["turns"].as_array().unwrap().len(), 1);
        assert_eq!(thread["turns"][0]["items"][0]["type"], "userMessage");
        assert_eq!(thread["turns"][0]["items"][1]["type"], "agentMessage");
        assert_eq!(thread["turns"][0]["items"][1]["text"], "pong");
    }

    #[test]
    fn forbids_theseus_app_server_methods() {
        assert!(is_theseus_app_server_method("thread/start"));
        assert!(is_theseus_app_server_method("tool/approve"));
        assert!(is_pi_command("new_session"));
        assert!(is_pi_command("prompt"));
        assert!(!is_pi_command("thread/start"));
        let _ = ev(json!({"type":"turn_end"}));
    }
}
