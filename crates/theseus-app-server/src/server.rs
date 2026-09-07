use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};
use theseus_core::{
    default_sessions_dir, list_session_files, preview_from_events, project_item, read_jsonl,
    session_log_path, FenceError, Session, SessionError, DEFAULT_MAX_STEPS_PER_TURN,
};
use theseus_llm::{
    default_model, messages_from_derived, ChatRequest, LlmError, LlmSeam, OpenAiChatSeam,
    OpenAiTool,
};
use theseus_protocol::{
    join_user_input, AssistantMessageBody, AssistantMessageEvent, EventData, SessionEvent, Thread,
    ThreadMeta, ThreadStatus, ThreadSummary, ToolCallEvent, ToolResultEvent, Turn, TurnEndReason,
    TurnStatus, UserInput, UserMessageEvent,
};
use theseus_tools::{tool_definitions, LocalToolSeam, ToolContext, ToolOutput, ToolSeam};

use crate::approval::{approval_summary, parse_arguments, timeout_from_env, ApprovalMode};
use crate::rpc::{JsonRpcError, JsonRpcId, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

#[derive(Debug, Clone)]
pub enum Outgoing {
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

impl Outgoing {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Response(r) => serde_json::to_value(r).expect("response"),
            Self::Notification(n) => serde_json::to_value(n).expect("notification"),
        }
    }

    pub fn to_line(&self) -> String {
        serde_json::to_string(&self.to_json()).expect("line")
    }
}

struct Out<'a> {
    buf: &'a mut Vec<Outgoing>,
    sink: &'a mut dyn FnMut(&Outgoing),
}

impl Out<'_> {
    fn push(&mut self, msg: Outgoing) {
        (self.sink)(&msg);
        self.buf.push(msg);
    }
}

#[derive(Debug, Default)]
pub struct HandleResult {
    pub outgoing: Vec<Outgoing>,
    pub shutdown: bool,
}

struct ThreadState {
    session: Session,
    thread: Thread,
    next_item: u64,
    model: String,
    workspace: PathBuf,
}

#[derive(Clone, Debug)]
struct QueuedCall {
    call_id: String,
    name: String,
    arguments: String,
}

#[derive(Clone, Debug)]
struct PendingApproval {
    thread_id: String,
    user_text: String,
    turn_id: String,
    turn_number: u32,
    step: u32,
    workspace: PathBuf,
    call_id: String,
    name: String,
    arguments: String,
    remaining: Vec<QueuedCall>,
    deadline: Instant,
    expires_at_ms: i64,
}

enum QueueOutcome {
    Done,
    Parked,
    Failed,
}

enum ApprovalDecision {
    Approve,
    Reject,
}

/// In-memory Codex-shaped app-server. Model calls go through [`LlmSeam`].
pub struct AppServer {
    initialized: bool,
    threads: HashMap<String, ThreadState>,
    next_thread: u64,
    llm: Box<dyn LlmSeam>,
    tools: Box<dyn ToolSeam>,
    max_steps_per_turn: u32,
    sessions_dir: PathBuf,
    approval: ApprovalMode,
    approval_timeout: Duration,
    pending: Option<PendingApproval>,
}

impl Default for AppServer {
    fn default() -> Self {
        Self::new()
    }
}

impl AppServer {
    pub fn new() -> Self {
        Self::with_llm(OpenAiChatSeam::from_env())
    }

    pub fn with_llm(llm: impl LlmSeam + 'static) -> Self {
        Self::with_seams(llm, LocalToolSeam, DEFAULT_MAX_STEPS_PER_TURN)
    }

    pub fn with_seams(
        llm: impl LlmSeam + 'static,
        tools: impl ToolSeam + 'static,
        max_steps_per_turn: u32,
    ) -> Self {
        Self {
            initialized: false,
            threads: HashMap::new(),
            next_thread: 0,
            llm: Box::new(llm),
            tools: Box::new(tools),
            max_steps_per_turn: max_steps_per_turn.max(1),
            sessions_dir: default_sessions_dir(),
            approval: ApprovalMode::from_env(),
            approval_timeout: timeout_from_env(),
            pending: None,
        }
    }

    /// Override the JSONL directory (`{dir}/{thread_id}.jsonl`). Shared by stdio and `theseus-web`.
    pub fn with_sessions_dir(mut self, dir: PathBuf) -> Self {
        self.sessions_dir = dir;
        self
    }

    /// Process-level gate: `auto` runs every tool; `approve` waits on write/edit/bash.
    pub fn with_approval(mut self, mode: ApprovalMode, timeout: Duration) -> Self {
        self.approval = mode;
        self.approval_timeout = timeout;
        self
    }

    pub fn sessions_dir(&self) -> &std::path::Path {
        &self.sessions_dir
    }

    pub fn handle_line(&mut self, line: &str) -> HandleResult {
        self.handle_line_sink(line, &mut |_| {})
    }

    pub fn handle_line_sink(
        &mut self,
        line: &str,
        sink: &mut dyn FnMut(&Outgoing),
    ) -> HandleResult {
        let line = line.trim();
        if line.is_empty() {
            return HandleResult::default();
        }
        match serde_json::from_str::<JsonRpcRequest>(line) {
            Ok(req) => self.handle_request_sink(req, sink),
            Err(err) => {
                let mut outgoing = Vec::new();
                let mut out = Out {
                    buf: &mut outgoing,
                    sink,
                };
                out.push(Outgoing::Response(JsonRpcResponse::err(
                    JsonRpcId::Null,
                    JsonRpcError::parse_error(err.to_string()),
                )));
                HandleResult {
                    outgoing,
                    shutdown: false,
                }
            }
        }
    }

    pub fn handle_request(&mut self, req: JsonRpcRequest) -> HandleResult {
        self.handle_request_sink(req, &mut |_| {})
    }

    fn handle_request_sink(
        &mut self,
        req: JsonRpcRequest,
        sink: &mut dyn FnMut(&Outgoing),
    ) -> HandleResult {
        if req.method == "initialized" {
            return HandleResult::default();
        }

        let mut outgoing = Vec::new();
        let mut out = Out {
            buf: &mut outgoing,
            sink,
        };

        if !matches!(req.method.as_str(), "tool/approve" | "tool/reject") {
            self.expire_pending_if_needed(&mut out);
        }

        let Some(id) = req.id.clone() else {
            return HandleResult::default();
        };

        let shutdown = match req.method.as_str() {
            "initialize" => {
                self.initialize(&mut out, id, &req.params);
                false
            }
            "shutdown" => {
                out.push(Outgoing::Response(JsonRpcResponse::ok(id, json!({}))));
                true
            }
            "thread/start" => {
                if !self.initialized {
                    push_error(&mut out, id, JsonRpcError::not_initialized());
                } else {
                    self.thread_start(&mut out, id, &req.params);
                }
                false
            }
            "thread/resume" => {
                if !self.initialized {
                    push_error(&mut out, id, JsonRpcError::not_initialized());
                } else {
                    self.thread_resume(&mut out, id, &req.params);
                }
                false
            }
            "thread/list" => {
                if !self.initialized {
                    push_error(&mut out, id, JsonRpcError::not_initialized());
                } else {
                    self.thread_list(&mut out, id, &req.params);
                }
                false
            }
            "turn/start" => {
                if !self.initialized {
                    push_error(&mut out, id, JsonRpcError::not_initialized());
                } else {
                    self.turn_start(&mut out, id, &req.params);
                }
                false
            }
            "tool/approve" => {
                if !self.initialized {
                    push_error(&mut out, id, JsonRpcError::not_initialized());
                } else {
                    self.tool_decide(&mut out, id, &req.params, ApprovalDecision::Approve);
                }
                false
            }
            "tool/reject" => {
                if !self.initialized {
                    push_error(&mut out, id, JsonRpcError::not_initialized());
                } else {
                    self.tool_decide(&mut out, id, &req.params, ApprovalDecision::Reject);
                }
                false
            }
            other => {
                push_error(&mut out, id, JsonRpcError::method_not_found(other));
                false
            }
        };

        HandleResult { outgoing, shutdown }
    }

    fn initialize(&mut self, out: &mut Out<'_>, id: JsonRpcId, _params: &Value) {
        if self.initialized {
            push_error(out, id, JsonRpcError::already_initialized());
            return;
        }
        self.initialized = true;
        out.push(Outgoing::Response(JsonRpcResponse::ok(
            id,
            json!({
                "userAgent": "theseus-app-server/0.1.0",
                "platformFamily": "unix",
                "platformOs": std::env::consts::OS,
            }),
        )));
    }

    fn thread_start(&mut self, out: &mut Out<'_>, id: JsonRpcId, params: &Value) {
        let params: ThreadStartParams = match serde_json::from_value(params.clone()) {
            Ok(p) => p,
            Err(err) => {
                push_error(out, id, JsonRpcError::invalid_params(err.to_string()));
                return;
            }
        };

        let thread_id = self.allocate_thread_id();
        let now_ms = now_ms();
        let created_at_secs = now_ms / 1000;
        let model = params
            .model
            .filter(|s| !s.is_empty())
            .unwrap_or_else(default_model);
        let cwd = params.cwd;

        let meta = ThreadMeta {
            thread_id: thread_id.clone(),
            model: Some(model.clone()),
            cwd: cwd.clone(),
            title: None,
            created_at: now_ms,
            parent_thread_id: None,
        };

        let workspace = resolve_workspace(cwd.as_deref());
        let session =
            match Session::create_persisted(meta, self.max_steps_per_turn, &self.sessions_dir) {
                Ok(s) => s,
                Err(err) => {
                    push_error(out, id, JsonRpcError::application(err.to_string()));
                    return;
                }
            };
        let meta_event = session.events()[0].clone();
        let path = session.log_path().map(|p| {
            std::fs::canonicalize(p)
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .into_owned()
        });

        let thread = Thread {
            id: thread_id.clone(),
            preview: String::new(),
            model_provider: "openai-compat".into(),
            created_at: created_at_secs,
            updated_at: created_at_secs,
            status: ThreadStatus::Idle,
            ephemeral: false,
            path,
            cwd,
            forked_from_id: None,
            turns: vec![],
        };

        self.threads.insert(
            thread_id,
            ThreadState {
                session,
                thread: thread.clone(),
                next_item: 0,
                model: model.clone(),
                workspace,
            },
        );

        out.push(Outgoing::Response(JsonRpcResponse::ok(
            id,
            json!({
                "thread": thread,
                "model": model,
                "modelProvider": "openai-compat",
            }),
        )));
        out.push(notify("thread/started", json!({ "thread": thread })));
        push_session_event(out, &meta_event);
    }

    fn thread_resume(&mut self, out: &mut Out<'_>, id: JsonRpcId, params: &Value) {
        let params: ThreadResumeParams = match serde_json::from_value(params.clone()) {
            Ok(p) => p,
            Err(err) => {
                push_error(out, id, JsonRpcError::invalid_params(err.to_string()));
                return;
            }
        };
        let thread_id = params.thread_id.filter(|s| !s.is_empty());
        let path = params.path.filter(|s| !s.is_empty());
        if thread_id.is_none() && path.is_none() {
            push_error(
                out,
                id,
                JsonRpcError::invalid_params("threadId or path is required"),
            );
            return;
        }

        let (want_id, file_path) =
            match self.resolve_resume_target(thread_id.as_deref(), path.as_deref()) {
                Ok(v) => v,
                Err(err) => {
                    push_error(out, id, JsonRpcError::application(err));
                    return;
                }
            };

        if self.threads.contains_key(&want_id) {
            let (thread, model) = {
                let state = self.threads.get_mut(&want_id).unwrap();
                state.thread = thread_from_session(&state.session);
                (state.thread.clone(), state.model.clone())
            };
            push_resume(out, id, &thread, &model);
            if let Some(pending) = self.pending.clone() {
                if pending.thread_id == want_id {
                    emit_approval_request(out, &pending);
                }
            }
            return;
        }

        let session = match Session::load_jsonl_with_max(file_path, self.max_steps_per_turn) {
            Ok(s) => s,
            Err(err) => {
                push_error(out, id, map_session(err));
                return;
            }
        };
        if session.id() != want_id {
            push_error(
                out,
                id,
                JsonRpcError::application(format!(
                    "session file id {} does not match {want_id}",
                    session.id()
                )),
            );
            return;
        }

        let model = session
            .events()
            .iter()
            .find_map(|e| match &e.data {
                EventData::ThreadMeta(m) => m.model.clone(),
                _ => None,
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(default_model);
        let cwd = session.events().iter().find_map(|e| match &e.data {
            EventData::ThreadMeta(m) => m.cwd.clone(),
            _ => None,
        });
        let workspace = resolve_workspace(cwd.as_deref());
        let next_item = next_item_from_session(&session);
        let thread = thread_from_session(&session);
        self.note_thread_id(&thread.id);
        self.threads.insert(
            thread.id.clone(),
            ThreadState {
                session,
                thread: thread.clone(),
                next_item,
                model: model.clone(),
                workspace,
            },
        );
        push_resume(out, id, &thread, &model);
    }

    fn thread_list(&mut self, out: &mut Out<'_>, id: JsonRpcId, params: &Value) {
        let params: ThreadListParams = match serde_json::from_value(params.clone()) {
            Ok(p) => p,
            Err(err) => {
                push_error(out, id, JsonRpcError::invalid_params(err.to_string()));
                return;
            }
        };
        let limit = params
            .limit
            .unwrap_or(DEFAULT_THREAD_LIST_LIMIT)
            .clamp(1, MAX_THREAD_LIST_LIMIT);
        let mut threads = Vec::new();
        for file in list_session_files(&self.sessions_dir)
            .into_iter()
            .take(limit as usize)
        {
            let preview = match read_jsonl(&file.path) {
                Ok(events) => preview_from_events(&events, 80),
                Err(_) => continue,
            };
            threads.push(ThreadSummary {
                id: file.thread_id,
                path: file.path.to_string_lossy().into_owned(),
                updated_at: file.updated_at,
                preview,
            });
        }
        out.push(Outgoing::Response(JsonRpcResponse::ok(
            id,
            json!({ "threads": threads, "limit": limit }),
        )));
    }

    fn allocate_thread_id(&mut self) -> String {
        for file in list_session_files(&self.sessions_dir) {
            self.note_thread_id(&file.thread_id);
        }
        loop {
            self.next_thread += 1;
            let id = format!("thr_{}", self.next_thread);
            let taken = session_log_path(&self.sessions_dir, &id)
                .map(|p| p.exists())
                .unwrap_or(true)
                || self.threads.contains_key(&id);
            if !taken {
                return id;
            }
        }
    }

    fn note_thread_id(&mut self, id: &str) {
        if let Some(n) = id.strip_prefix("thr_").and_then(|s| s.parse::<u64>().ok()) {
            self.next_thread = self.next_thread.max(n);
        }
    }

    fn resolve_resume_target(
        &self,
        thread_id: Option<&str>,
        path: Option<&str>,
    ) -> Result<(String, PathBuf), String> {
        let root =
            std::fs::canonicalize(&self.sessions_dir).unwrap_or_else(|_| self.sessions_dir.clone());
        if let Some(tid) = thread_id {
            let expected = session_log_path(&self.sessions_dir, tid).map_err(|e| e.to_string())?;
            if let Some(raw) = path {
                let got = constrain_session_path(raw, &self.sessions_dir, &root)?;
                let exp = std::fs::canonicalize(&expected).unwrap_or(expected);
                if got != exp {
                    return Err("path does not match threadId".into());
                }
                return Ok((tid.to_string(), got));
            }
            if !expected.exists() {
                return Err(format!("unknown thread {tid}"));
            }
            return Ok((
                tid.to_string(),
                std::fs::canonicalize(&expected).unwrap_or(expected),
            ));
        }
        if let Some(raw) = path {
            let got = constrain_session_path(raw, &self.sessions_dir, &root)?;
            let stem = got
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| "not a session jsonl".to_string())?;
            session_log_path(&self.sessions_dir, stem).map_err(|e| e.to_string())?;
            return Ok((stem.to_string(), got));
        }
        Err("threadId or path is required".into())
    }

    fn turn_start(&mut self, out: &mut Out<'_>, id: JsonRpcId, params: &Value) {
        if self.pending.is_some() {
            push_error(
                out,
                id,
                JsonRpcError::application(
                    "a tool is waiting for approval; send tool/approve or tool/reject first",
                ),
            );
            return;
        }

        let params: TurnStartParams = match serde_json::from_value(params.clone()) {
            Ok(p) => p,
            Err(err) => {
                push_error(out, id, JsonRpcError::invalid_params(err.to_string()));
                return;
            }
        };

        let text = join_user_input(&params.input);
        if text.is_empty() {
            push_error(out, id, JsonRpcError::invalid_params("input is empty"));
            return;
        }

        if let Err(err) = self.llm.ready() {
            push_error(out, id, map_llm(err));
            return;
        }

        let thread_id = params.thread_id.clone();
        if !self.threads.contains_key(&thread_id) {
            push_error(
                out,
                id,
                JsonRpcError::application(format!("unknown thread {}", params.thread_id)),
            );
            return;
        }

        let prepared = match prepare_turn(self.threads.get_mut(&thread_id).unwrap(), out, id, &text)
        {
            Some(p) => p,
            None => return,
        };

        self.run_agent_loop(out, &thread_id, &prepared, &text);
    }

    fn run_agent_loop(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        prepared: &PreparedTurn,
        user_text: &str,
    ) {
        loop {
            let (request, assistant_id, workspace) = {
                let Some(state) = self.threads.get_mut(thread_id) else {
                    return;
                };
                let assistant_id = next_item_id(state);
                let request = ChatRequest {
                    model: state.model.clone(),
                    messages: messages_from_derived(&state.session.derive_messages()),
                    tools: chat_tools(),
                };
                (request, assistant_id, state.workspace.clone())
            };

            let stream_result = {
                let assistant_id = assistant_id.clone();
                self.llm.stream_chat(&request, &mut |delta| {
                    if !delta.is_empty() {
                        out.push(notify(
                            "item/agentMessage/delta",
                            json!({
                                "itemId": assistant_id,
                                "delta": delta,
                            }),
                        ));
                    }
                })
            };

            let Some(state) = self.threads.get_mut(thread_id) else {
                return;
            };

            let outcome = match stream_result {
                Ok(o) => o,
                Err(err) => {
                    close_failed_turn(state, out, &prepared.turn_id, prepared.turn_number, err);
                    return;
                }
            };

            let step = match state.session.open_step() {
                Some((_, s)) => s,
                None => {
                    close_turn(
                        state,
                        out,
                        &prepared.turn_id,
                        prepared.turn_number,
                        TurnEndReason::Error {
                            message: "internal: missing open step".into(),
                        },
                    );
                    return;
                }
            };

            if outcome.tool_calls.is_empty() {
                if outcome.text.is_empty() {
                    close_failed_turn(
                        state,
                        out,
                        &prepared.turn_id,
                        prepared.turn_number,
                        LlmError::InvalidResponse("empty model response".into()),
                    );
                    return;
                }
                self.finish_text_turn(
                    out,
                    thread_id,
                    prepared,
                    user_text,
                    assistant_id,
                    step,
                    outcome.text,
                );
                return;
            }

            // Tool path: never copy calls onto assistant/message.
            if !outcome.text.is_empty() {
                match state
                    .session
                    .append(EventData::AssistantMessage(AssistantMessageEvent {
                        turn: prepared.turn_number,
                        step,
                        message: AssistantMessageBody::text(assistant_id, outcome.text),
                        usage: None,
                        interrupted: None,
                    })) {
                    Ok(ev) => push_session_event(out, &ev),
                    Err(err) => {
                        close_failed_turn(
                            state,
                            out,
                            &prepared.turn_id,
                            prepared.turn_number,
                            LlmError::InvalidResponse(err.to_string()),
                        );
                        return;
                    }
                }
            }

            let calls: Vec<QueuedCall> = outcome
                .tool_calls
                .into_iter()
                .enumerate()
                .map(|(i, call)| QueuedCall {
                    call_id: if call.id.is_empty() {
                        format!("call_{}_{}_{}", prepared.turn_number, step, i + 1)
                    } else {
                        call.id
                    },
                    name: call.name,
                    arguments: call.arguments,
                })
                .collect();

            match self.run_tool_queue(out, thread_id, prepared, user_text, step, &workspace, calls)
            {
                QueueOutcome::Parked => return,
                QueueOutcome::Failed => return,
                QueueOutcome::Done => {
                    if !self.finish_step_or_cap(out, thread_id, prepared) {
                        return;
                    }
                }
            }
        }
    }

    fn finish_text_turn(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        prepared: &PreparedTurn,
        user_text: &str,
        assistant_id: String,
        step: u32,
        content: String,
    ) {
        let Some(state) = self.threads.get_mut(thread_id) else {
            return;
        };
        let assistant_ev =
            match state
                .session
                .append(EventData::AssistantMessage(AssistantMessageEvent {
                    turn: prepared.turn_number,
                    step,
                    message: AssistantMessageBody::text(assistant_id, content),
                    usage: None,
                    interrupted: None,
                })) {
                Ok(ev) => ev,
                Err(err) => {
                    close_failed_turn(
                        state,
                        out,
                        &prepared.turn_id,
                        prepared.turn_number,
                        LlmError::InvalidResponse(err.to_string()),
                    );
                    return;
                }
            };

        let step_end_ev = match state.session.end_step() {
            Ok(ev) => ev,
            Err(err) => {
                close_turn(
                    state,
                    out,
                    &prepared.turn_id,
                    prepared.turn_number,
                    TurnEndReason::Error {
                        message: err.to_string(),
                    },
                );
                return;
            }
        };
        let turn_end_ev = match state.session.end_turn(TurnEndReason::Completed) {
            Ok(ev) => ev,
            Err(err) => {
                close_turn(
                    state,
                    out,
                    &prepared.turn_id,
                    prepared.turn_number,
                    TurnEndReason::Error {
                        message: err.to_string(),
                    },
                );
                return;
            }
        };

        let completed_turn = Turn {
            id: prepared.turn_id.clone(),
            status: TurnStatus::Completed,
            items: state.session.project_items_for_turn(prepared.turn_number),
        };
        state.thread.status = ThreadStatus::Idle;
        state.thread.updated_at = now_ms() / 1000;
        state.thread.preview = preview_of(user_text);
        state.thread.turns.push(completed_turn.clone());

        push_session_event(out, &assistant_ev);
        push_session_event(out, &step_end_ev);
        push_session_event(out, &turn_end_ev);
        out.push(notify("turn/completed", json!({ "turn": completed_turn })));
    }

    fn run_tool_queue(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        prepared: &PreparedTurn,
        user_text: &str,
        step: u32,
        workspace: &Path,
        calls: Vec<QueuedCall>,
    ) -> QueueOutcome {
        let mut iter = calls.into_iter();
        while let Some(call) = iter.next() {
            if !self.append_tool_call(out, thread_id, prepared, step, &call) {
                return QueueOutcome::Failed;
            }
            if self.approval.requires_approval(&call.name) {
                self.park_approval(
                    out,
                    thread_id,
                    user_text,
                    prepared,
                    step,
                    workspace,
                    call,
                    iter.collect(),
                );
                return QueueOutcome::Parked;
            }
            if !self.execute_and_append(out, thread_id, prepared, step, workspace, &call) {
                return QueueOutcome::Failed;
            }
        }
        QueueOutcome::Done
    }

    fn park_approval(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        user_text: &str,
        prepared: &PreparedTurn,
        step: u32,
        workspace: &Path,
        call: QueuedCall,
        remaining: Vec<QueuedCall>,
    ) {
        let timeout_ms = i64::try_from(self.approval_timeout.as_millis()).unwrap_or(i64::MAX);
        let pending = PendingApproval {
            thread_id: thread_id.to_string(),
            user_text: user_text.to_string(),
            turn_id: prepared.turn_id.clone(),
            turn_number: prepared.turn_number,
            step,
            workspace: workspace.to_path_buf(),
            call_id: call.call_id,
            name: call.name,
            arguments: call.arguments,
            remaining,
            deadline: Instant::now() + self.approval_timeout,
            expires_at_ms: now_ms().saturating_add(timeout_ms),
        };
        emit_approval_request(out, &pending);
        self.pending = Some(pending);
    }

    fn expire_pending_if_needed(&mut self, out: &mut Out<'_>) {
        let Some(pending) = &self.pending else {
            return;
        };
        if Instant::now() < pending.deadline {
            return;
        }
        let thread_id = pending.thread_id.clone();
        let call_id = pending.call_id.clone();
        let _ = self.settle_pending(out, &thread_id, &call_id, ApprovalDecision::Reject);
    }

    fn tool_decide(
        &mut self,
        out: &mut Out<'_>,
        id: JsonRpcId,
        params: &Value,
        decision: ApprovalDecision,
    ) {
        let params: ToolDecisionParams = match serde_json::from_value(params.clone()) {
            Ok(p) => p,
            Err(err) => {
                push_error(out, id, JsonRpcError::invalid_params(err.to_string()));
                return;
            }
        };
        match self.settle_pending(out, &params.thread_id, &params.call_id, decision) {
            Ok(label) => {
                out.push(Outgoing::Response(JsonRpcResponse::ok(
                    id,
                    json!({ "decision": label }),
                )));
            }
            Err(err) => push_error(out, id, JsonRpcError::application(err)),
        }
    }

    fn settle_pending(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        call_id: &str,
        decision: ApprovalDecision,
    ) -> Result<&'static str, String> {
        let pending = self
            .pending
            .as_ref()
            .ok_or_else(|| "no pending tool approval".to_string())?;
        if pending.thread_id != thread_id {
            return Err(format!(
                "pending approval is for thread {}",
                pending.thread_id
            ));
        }
        if pending.call_id != call_id {
            return Err(format!("pending approval is for call {}", pending.call_id));
        }
        let expired = Instant::now() >= pending.deadline;
        let pending = self.pending.take().expect("pending checked");
        let prepared = PreparedTurn {
            turn_id: pending.turn_id.clone(),
            turn_number: pending.turn_number,
        };

        let (output, label) = if expired {
            (ToolOutput::error("tool approval timed out"), "timeout")
        } else {
            match decision {
                ApprovalDecision::Approve => (
                    self.tools.execute(
                        &pending.name,
                        &pending.arguments,
                        &ToolContext {
                            cwd: pending.workspace.clone(),
                        },
                    ),
                    "approved",
                ),
                ApprovalDecision::Reject => (ToolOutput::error("tool call rejected"), "rejected"),
            }
        };

        out.push(notify(
            "item/tool/approval/resolved",
            json!({
                "threadId": pending.thread_id,
                "callId": pending.call_id,
                "decision": label,
            }),
        ));

        if !self.append_tool_result(
            out,
            &pending.thread_id,
            &prepared,
            pending.step,
            &pending.call_id,
            output,
        ) {
            return Ok(label);
        }

        match self.run_tool_queue(
            out,
            &pending.thread_id,
            &prepared,
            &pending.user_text,
            pending.step,
            &pending.workspace,
            pending.remaining,
        ) {
            QueueOutcome::Parked | QueueOutcome::Failed => Ok(label),
            QueueOutcome::Done => {
                if self.finish_step_or_cap(out, &pending.thread_id, &prepared) {
                    self.run_agent_loop(out, &pending.thread_id, &prepared, &pending.user_text);
                }
                Ok(label)
            }
        }
    }

    fn append_tool_call(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        prepared: &PreparedTurn,
        step: u32,
        call: &QueuedCall,
    ) -> bool {
        let Some(state) = self.threads.get_mut(thread_id) else {
            return false;
        };
        match state.session.append(EventData::ToolCall(ToolCallEvent {
            turn: prepared.turn_number,
            step,
            call_id: call.call_id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        })) {
            Ok(ev) => {
                push_session_event(out, &ev);
                true
            }
            Err(err) => {
                close_turn(
                    state,
                    out,
                    &prepared.turn_id,
                    prepared.turn_number,
                    TurnEndReason::Error {
                        message: err.to_string(),
                    },
                );
                false
            }
        }
    }

    fn execute_and_append(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        prepared: &PreparedTurn,
        step: u32,
        workspace: &Path,
        call: &QueuedCall,
    ) -> bool {
        let output = self.tools.execute(
            &call.name,
            &call.arguments,
            &ToolContext {
                cwd: workspace.to_path_buf(),
            },
        );
        self.append_tool_result(out, thread_id, prepared, step, &call.call_id, output)
    }

    fn append_tool_result(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        prepared: &PreparedTurn,
        step: u32,
        call_id: &str,
        output: ToolOutput,
    ) -> bool {
        let Some(state) = self.threads.get_mut(thread_id) else {
            return false;
        };
        match state.session.append(EventData::ToolResult(ToolResultEvent {
            turn: prepared.turn_number,
            step,
            call_id: call_id.to_string(),
            content: output.content,
            is_error: output.is_error,
        })) {
            Ok(ev) => {
                push_session_event(out, &ev);
                true
            }
            Err(err) => {
                close_turn(
                    state,
                    out,
                    &prepared.turn_id,
                    prepared.turn_number,
                    TurnEndReason::Error {
                        message: err.to_string(),
                    },
                );
                false
            }
        }
    }

    fn finish_step_or_cap(
        &mut self,
        out: &mut Out<'_>,
        thread_id: &str,
        prepared: &PreparedTurn,
    ) -> bool {
        let Some(state) = self.threads.get_mut(thread_id) else {
            return false;
        };
        match state.session.end_step() {
            Ok(ev) => push_session_event(out, &ev),
            Err(err) => {
                close_turn(
                    state,
                    out,
                    &prepared.turn_id,
                    prepared.turn_number,
                    TurnEndReason::Error {
                        message: err.to_string(),
                    },
                );
                return false;
            }
        }

        match state.session.start_step() {
            Ok(ev) => {
                push_session_event(out, &ev);
                true
            }
            Err(SessionError::Fence(FenceError::MaxStepsReached { max, .. })) => {
                close_turn(
                    state,
                    out,
                    &prepared.turn_id,
                    prepared.turn_number,
                    TurnEndReason::Aborted {
                        reason: Some(format!("max steps reached ({max})")),
                    },
                );
                false
            }
            Err(err) => {
                close_turn(
                    state,
                    out,
                    &prepared.turn_id,
                    prepared.turn_number,
                    TurnEndReason::Error {
                        message: err.to_string(),
                    },
                );
                false
            }
        }
    }
}

#[cfg(test)]
impl AppServer {
    fn event_types(&self, thread_id: &str) -> Vec<String> {
        self.threads
            .get(thread_id)
            .map(|s| {
                s.session
                    .events()
                    .iter()
                    .map(|e| e.type_name().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn derived(&self, thread_id: &str) -> Vec<theseus_protocol::DerivedMessage> {
        self.threads
            .get(thread_id)
            .map(|s| s.session.derive_messages())
            .unwrap_or_default()
    }

    fn workspace(&self, thread_id: &str) -> PathBuf {
        self.threads
            .get(thread_id)
            .map(|s| s.workspace.clone())
            .unwrap_or_default()
    }

    fn events(&self, thread_id: &str) -> Vec<SessionEvent> {
        self.threads
            .get(thread_id)
            .map(|s| s.session.events().to_vec())
            .unwrap_or_default()
    }

    fn thread_path(&self, thread_id: &str) -> Option<String> {
        self.threads
            .get(thread_id)
            .and_then(|s| s.thread.path.clone())
    }

    fn pending_call_id(&self) -> Option<String> {
        self.pending.as_ref().map(|p| p.call_id.clone())
    }

    fn turn_is_open(&self, thread_id: &str) -> bool {
        self.threads
            .get(thread_id)
            .map(|s| s.session.open_turn().is_some())
            .unwrap_or(false)
    }

    fn step_is_open(&self, thread_id: &str) -> bool {
        self.threads
            .get(thread_id)
            .map(|s| s.session.open_step().is_some())
            .unwrap_or(false)
    }
}

struct PreparedTurn {
    turn_number: u32,
    turn_id: String,
}

fn prepare_turn(
    state: &mut ThreadState,
    out: &mut Out<'_>,
    id: JsonRpcId,
    text: &str,
) -> Option<PreparedTurn> {
    if state.session.open_turn().is_some() {
        push_error(
            out,
            id,
            JsonRpcError::application("a turn is already in progress"),
        );
        return None;
    }

    let turn_start_ev = match state.session.start_turn() {
        Ok(ev) => ev,
        Err(err) => {
            push_error(out, id, map_session(err));
            return None;
        }
    };
    let turn_number = match &turn_start_ev.data {
        EventData::TurnStart(t) => t.turn,
        _ => unreachable!(),
    };
    let turn_id = format!("turn_{}_{turn_number}", state.thread.id);

    let user_id = next_item_id(state);
    let user_ev = match state
        .session
        .append(EventData::UserMessage(UserMessageEvent {
            turn: turn_number,
            id: user_id,
            content: text.to_string(),
            source: Some("user".into()),
        })) {
        Ok(ev) => ev,
        Err(err) => {
            let message = err.to_string();
            let _ = state.session.end_turn(TurnEndReason::Error {
                message: message.clone(),
            });
            push_error(out, id, map_session(err));
            return None;
        }
    };

    let step_start_ev = match state.session.start_step() {
        Ok(ev) => ev,
        Err(err) => {
            let message = err.to_string();
            if state.session.open_step().is_some() {
                let _ = state.session.end_step();
            }
            let _ = state.session.end_turn(TurnEndReason::Error { message });
            push_error(out, id, map_session(err));
            return None;
        }
    };

    let initial_turn = Turn {
        id: turn_id.clone(),
        status: TurnStatus::InProgress,
        items: vec![],
    };
    out.push(Outgoing::Response(JsonRpcResponse::ok(
        id,
        json!({ "turn": initial_turn }),
    )));
    push_session_event(out, &turn_start_ev);
    out.push(notify("turn/started", json!({ "turn": initial_turn })));
    push_session_event(out, &user_ev);
    push_session_event(out, &step_start_ev);

    Some(PreparedTurn {
        turn_number,
        turn_id,
    })
}

fn close_failed_turn(
    state: &mut ThreadState,
    out: &mut Out<'_>,
    turn_id: &str,
    turn_number: u32,
    err: LlmError,
) {
    close_turn(
        state,
        out,
        turn_id,
        turn_number,
        TurnEndReason::Error {
            message: err.to_string(),
        },
    );
}

fn close_turn(
    state: &mut ThreadState,
    out: &mut Out<'_>,
    turn_id: &str,
    turn_number: u32,
    reason: TurnEndReason,
) {
    if state.session.open_step().is_some() {
        if let Ok(ev) = state.session.end_step() {
            push_session_event(out, &ev);
        }
    }
    if state.session.open_turn().is_some() {
        if let Ok(ev) = state.session.end_turn(reason) {
            push_session_event(out, &ev);
        }
    }
    let failed = Turn {
        id: turn_id.to_string(),
        status: TurnStatus::Failed,
        items: state.session.project_items_for_turn(turn_number),
    };
    state.thread.status = ThreadStatus::Idle;
    state.thread.updated_at = now_ms() / 1000;
    state.thread.turns.push(failed.clone());
    out.push(notify("turn/completed", json!({ "turn": failed })));
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadStartParams {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    /// Accepted for wire compat. Threads are always persisted; this is ignored.
    #[serde(default)]
    #[allow(dead_code)]
    ephemeral: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadResumeParams {
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadListParams {
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnStartParams {
    thread_id: String,
    #[serde(default)]
    input: Vec<UserInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolDecisionParams {
    thread_id: String,
    call_id: String,
}

const DEFAULT_THREAD_LIST_LIMIT: u32 = 20;
const MAX_THREAD_LIST_LIMIT: u32 = 100;

fn next_item_id(state: &mut ThreadState) -> String {
    state.next_item += 1;
    format!("item_{}_{}", state.thread.id, state.next_item)
}

fn next_item_from_session(session: &Session) -> u64 {
    let prefix = format!("item_{}_", session.id());
    let mut max = 0u64;
    for ev in session.events() {
        let id = match &ev.data {
            EventData::UserMessage(m) => m.id.as_str(),
            EventData::AssistantMessage(m) => m.message.id.as_str(),
            _ => continue,
        };
        if let Some(rest) = id.strip_prefix(&prefix) {
            if let Ok(n) = rest.parse::<u64>() {
                max = max.max(n);
            }
        }
    }
    max
}

fn turns_from_session(session: &Session) -> Vec<Turn> {
    let mut turns = Vec::new();
    for ev in session.events() {
        match &ev.data {
            EventData::TurnStart(t) => {
                turns.push(Turn {
                    id: format!("turn_{}_{}", session.id(), t.turn),
                    status: TurnStatus::InProgress,
                    items: session.project_items_for_turn(t.turn),
                });
            }
            EventData::TurnEnd(t) => {
                let id = format!("turn_{}_{}", session.id(), t.turn);
                if let Some(turn) = turns.iter_mut().rev().find(|turn| turn.id == id) {
                    turn.status = match &t.reason {
                        TurnEndReason::Completed => TurnStatus::Completed,
                        TurnEndReason::Interrupted => TurnStatus::Interrupted,
                        _ => TurnStatus::Failed,
                    };
                    turn.items = session.project_items_for_turn(t.turn);
                }
            }
            _ => {}
        }
    }
    turns
}

fn thread_from_session(session: &Session) -> Thread {
    let meta = session.events().iter().find_map(|e| match &e.data {
        EventData::ThreadMeta(m) => Some(m),
        _ => None,
    });
    let created_at = meta.map(|m| m.created_at / 1000).unwrap_or(0);
    let updated_at = session
        .events()
        .last()
        .map(|e| e.time / 1000)
        .unwrap_or(created_at);
    let path = session.log_path().map(|p| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .into_owned()
    });
    Thread {
        id: session.id().to_string(),
        preview: session.preview(),
        model_provider: "openai-compat".into(),
        created_at,
        updated_at,
        status: if session.open_turn().is_some() {
            ThreadStatus::Active {
                active_flags: vec![],
            }
        } else {
            ThreadStatus::Idle
        },
        ephemeral: false,
        path,
        cwd: meta.and_then(|m| m.cwd.clone()),
        forked_from_id: meta.and_then(|m| m.parent_thread_id.clone()),
        turns: turns_from_session(session),
    }
}

fn constrain_session_path(raw: &str, sessions_dir: &Path, root: &Path) -> Result<PathBuf, String> {
    let given = PathBuf::from(raw);
    let candidate = if given.is_absolute() {
        given
    } else {
        sessions_dir.join(given)
    };
    let canon =
        std::fs::canonicalize(&candidate).map_err(|_| format!("session file not found: {raw}"))?;
    if !canon.starts_with(root) {
        return Err("path escapes sessions dir".into());
    }
    if canon.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        return Err("not a session jsonl".into());
    }
    Ok(canon)
}

fn push_resume(out: &mut Out<'_>, id: JsonRpcId, thread: &Thread, model: &str) {
    out.push(Outgoing::Response(JsonRpcResponse::ok(
        id,
        json!({
            "thread": thread,
            "model": model,
            "modelProvider": "openai-compat",
        }),
    )));
    out.push(notify("thread/started", json!({ "thread": thread })));
    for turn in &thread.turns {
        for item in &turn.items {
            out.push(notify("item/completed", json!({ "item": item })));
        }
    }
}

fn notify(method: &str, params: Value) -> Outgoing {
    Outgoing::Notification(JsonRpcNotification::new(method, params))
}

fn emit_approval_request(out: &mut Out<'_>, pending: &PendingApproval) {
    out.push(notify(
        "item/tool/approval/request",
        json!({
            "threadId": pending.thread_id,
            "turnId": pending.turn_id,
            "callId": pending.call_id,
            "tool": pending.name,
            "arguments": parse_arguments(&pending.arguments),
            "summary": approval_summary(&pending.name, &pending.arguments),
            "expiresAtMs": pending.expires_at_ms,
        }),
    ));
}

fn push_session_event(out: &mut Out<'_>, event: &SessionEvent) {
    out.push(notify("session/event", json!({ "event": event })));
    if let Some(item) = project_item(event) {
        out.push(notify("item/started", json!({ "item": item })));
        out.push(notify("item/completed", json!({ "item": item })));
    }
}

fn push_error(out: &mut Out<'_>, id: JsonRpcId, error: JsonRpcError) {
    out.push(Outgoing::Response(JsonRpcResponse::err(id, error)));
}

fn map_session(err: SessionError) -> JsonRpcError {
    JsonRpcError::application(err.to_string())
}

fn map_llm(err: LlmError) -> JsonRpcError {
    JsonRpcError::application(err.to_string())
}

fn preview_of(text: &str) -> String {
    text.chars().take(80).collect()
}

fn resolve_workspace(cwd: Option<&str>) -> PathBuf {
    match cwd {
        Some(p) if !p.is_empty() => {
            let path = PathBuf::from(p);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir()
                    .map(|base| base.join(path))
                    .unwrap_or_else(|_| PathBuf::from(p))
            }
        }
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

fn chat_tools() -> Vec<OpenAiTool> {
    tool_definitions()
        .into_iter()
        .map(|d| OpenAiTool::function(d.name, d.description, d.parameters))
        .collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use theseus_llm::{LlmError, ScriptedRound, ScriptedSeam, ToolCallRequest};
    use theseus_protocol::{DerivedMessage, ThreadItem};

    fn test_server(llm: impl LlmSeam + 'static) -> AppServer {
        AppServer::with_llm(llm)
            .with_sessions_dir(temp_sessions())
            .with_approval(ApprovalMode::Auto, Duration::from_secs(60))
    }

    fn test_seams(
        llm: impl LlmSeam + 'static,
        tools: impl ToolSeam + 'static,
        max: u32,
    ) -> AppServer {
        AppServer::with_seams(llm, tools, max)
            .with_sessions_dir(temp_sessions())
            .with_approval(ApprovalMode::Auto, Duration::from_secs(60))
    }

    fn approve_server(llm: impl LlmSeam + 'static, timeout: Duration) -> AppServer {
        AppServer::with_llm(llm)
            .with_sessions_dir(temp_sessions())
            .with_approval(ApprovalMode::Approve, timeout)
    }

    fn temp_sessions() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "theseus-app-sess-{}-{}-{}",
            std::process::id(),
            now_ms(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn req(id: i64, method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: Some("2.0".into()),
            method: method.into(),
            params,
            id: Some(JsonRpcId::Number(id)),
        }
    }

    fn start_thread(server: &mut AppServer) -> String {
        start_thread_with(server, json!({ "model": "gpt-4o-mini" }))
    }

    fn start_thread_with(server: &mut AppServer, params: Value) -> String {
        server.handle_request(req(
            0,
            "initialize",
            json!({ "clientInfo": { "name": "test", "version": "0.1.0" } }),
        ));
        let started = server.handle_request(req(1, "thread/start", params));
        started
            .outgoing
            .iter()
            .find_map(|o| match o {
                Outgoing::Response(r) => r
                    .result
                    .as_ref()?
                    .get("thread")?
                    .get("id")?
                    .as_str()
                    .map(str::to_string),
                _ => None,
            })
            .expect("thread id")
    }

    fn temp_workspace(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "theseus-app-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn call_read(path: &str) -> ToolCallRequest {
        ToolCallRequest {
            id: "c_read".into(),
            name: "read".into(),
            arguments: format!(r#"{{"path":"{path}"}}"#),
        }
    }

    fn session_event_types(turn: &HandleResult) -> Vec<String> {
        turn.outgoing
            .iter()
            .filter_map(|o| match o {
                Outgoing::Notification(n) if n.method == "session/event" => {
                    n.params["event"]["type"].as_str().map(str::to_string)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn thread_and_turn_emit_session_events() {
        let mut server = test_server(ScriptedSeam::ok(["hel", "lo"]));
        let thread_id = start_thread(&mut server);

        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }]
            }),
        ));

        let event_types = session_event_types(&turn);
        assert_eq!(
            event_types,
            vec![
                "turn/start",
                "user/message",
                "step/start",
                "assistant/message",
                "step/end",
                "turn/end",
            ]
        );
        assert!(!event_types.iter().any(|t| t == "assistant/chunk"));

        let methods: Vec<&str> = turn
            .outgoing
            .iter()
            .map(|o| match o {
                Outgoing::Response(_) => "response",
                Outgoing::Notification(n) => n.method.as_str(),
            })
            .collect();
        assert!(methods.contains(&"turn/started"));
        assert!(methods.contains(&"turn/completed"));
        assert!(methods.contains(&"item/agentMessage/delta"));

        let deltas: Vec<String> = turn
            .outgoing
            .iter()
            .filter_map(|o| match o {
                Outgoing::Notification(n) if n.method == "item/agentMessage/delta" => {
                    n.params["delta"].as_str().map(str::to_string)
                }
                _ => None,
            })
            .collect();
        assert_eq!(deltas, vec!["hel", "lo"]);

        let events: Vec<SessionEvent> = turn
            .outgoing
            .iter()
            .filter_map(|o| match o {
                Outgoing::Notification(n) if n.method == "session/event" => {
                    serde_json::from_value(n.params["event"].clone()).ok()
                }
                _ => None,
            })
            .collect();
        let projected = theseus_core::project_items(&events);
        let notified: Vec<ThreadItem> = turn
            .outgoing
            .iter()
            .filter_map(|o| match o {
                Outgoing::Notification(n) if n.method == "item/completed" => {
                    serde_json::from_value(n.params["item"].clone()).ok()
                }
                _ => None,
            })
            .collect();
        assert_eq!(notified, projected);
        let assistant = events.iter().find_map(|e| match &e.data {
            EventData::AssistantMessage(m) => Some(m.message.content.as_str()),
            _ => None,
        });
        assert_eq!(assistant, Some("hello"));
        assert_assistant_json_omits_tool_calls(&events);
    }

    #[test]
    fn no_key_is_rpc_error_and_does_not_pollute_log() {
        let mut server = test_server(ScriptedSeam::fail(LlmError::MissingApiKey));
        let thread_id = start_thread(&mut server);
        assert_eq!(server.event_types(&thread_id), vec!["thread/meta"]);

        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }]
            }),
        ));
        match &turn.outgoing[0] {
            Outgoing::Response(r) => {
                let msg = r.error.as_ref().unwrap().message.clone();
                assert!(msg.contains("THESEUS_LLM_API_KEY"), "{msg}");
            }
            _ => panic!("expected RPC error"),
        }
        assert!(session_event_types(&turn).is_empty());
        assert_eq!(server.event_types(&thread_id), vec!["thread/meta"]);
    }

    #[test]
    fn seam_failure_closes_turn_without_assistant_message() {
        let mut server = test_server(ScriptedSeam::fail(LlmError::Http {
            status: 500,
            message: "boom".into(),
        }));
        let thread_id = start_thread(&mut server);
        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }]
            }),
        ));
        let types = server.event_types(&thread_id);
        assert_eq!(
            types,
            vec![
                "thread/meta",
                "turn/start",
                "user/message",
                "step/start",
                "step/end",
                "turn/end",
            ]
        );
        assert!(!types.iter().any(|t| t == "assistant/message"));
        let end = turn.outgoing.iter().find_map(|o| match o {
            Outgoing::Notification(n) if n.method == "session/event" => {
                if n.params["event"]["type"] == "turn/end" {
                    Some(
                        n.params["event"]["data"]["reason"]["kind"]
                            .as_str()?
                            .to_string(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        });
        assert_eq!(end.as_deref(), Some("error"));
        assert!(!session_event_types(&turn)
            .iter()
            .any(|t| t == "assistant/message"));
    }

    #[test]
    fn empty_ok_does_not_append_assistant_message() {
        let mut server = test_server(ScriptedSeam::ok([""]));
        let thread_id = start_thread(&mut server);
        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }]
            }),
        ));
        assert_eq!(
            server.event_types(&thread_id),
            vec![
                "thread/meta",
                "turn/start",
                "user/message",
                "step/start",
                "step/end",
                "turn/end",
            ]
        );
        assert!(!server
            .event_types(&thread_id)
            .iter()
            .any(|t| t == "assistant/message"));
        assert!(!server.turn_is_open(&thread_id));
        assert!(!server.step_is_open(&thread_id));
        let end = turn.outgoing.iter().find_map(|o| match o {
            Outgoing::Notification(n) if n.method == "session/event" => {
                if n.params["event"]["type"] == "turn/end" {
                    Some(
                        n.params["event"]["data"]["reason"]["kind"]
                            .as_str()?
                            .to_string(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        });
        assert_eq!(end.as_deref(), Some("error"));
        assert!(!session_event_types(&turn)
            .iter()
            .any(|t| t == "assistant/message"));
        assert_eq!(
            server.derived(&thread_id),
            vec![DerivedMessage::User {
                content: "hello".into()
            }]
        );
    }

    #[test]
    fn rejects_work_before_initialize() {
        let mut server = test_server(ScriptedSeam::ok(["x"]));
        let out = server.handle_request(req(1, "thread/start", json!({})));
        match &out.outgoing[0] {
            Outgoing::Response(r) => {
                assert_eq!(r.error.as_ref().unwrap().message, "Not initialized");
            }
            _ => panic!("expected error response"),
        }
    }

    #[test]
    fn tool_loop_writes_then_replies_from_workspace() {
        let dir = temp_workspace("loop");
        let seam = ScriptedSeam::rounds([
            ScriptedRound::ToolCalls {
                text: String::new(),
                calls: vec![ToolCallRequest {
                    id: "c1".into(),
                    name: "write".into(),
                    arguments: r#"{"path":"out.txt","content":"from-tool"}"#.into(),
                }],
            },
            ScriptedRound::Text(vec!["wrote it".into()]),
        ]);
        let mut server = test_server(seam);
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        assert_eq!(server.workspace(&thread_id), dir);

        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "write out.txt" }]
            }),
        ));

        assert_eq!(
            session_event_types(&turn),
            vec![
                "turn/start",
                "user/message",
                "step/start",
                "tool/call",
                "tool/result",
                "step/end",
                "step/start",
                "assistant/message",
                "step/end",
                "turn/end",
            ]
        );

        let body = fs::read_to_string(dir.join("out.txt")).expect("file in thread cwd");
        assert_eq!(body, "from-tool");

        let events: Vec<SessionEvent> = turn
            .outgoing
            .iter()
            .filter_map(|o| match o {
                Outgoing::Notification(n) if n.method == "session/event" => {
                    serde_json::from_value(n.params["event"].clone()).ok()
                }
                _ => None,
            })
            .collect();
        assert_assistant_json_omits_tool_calls(&events);

        let derived = server.derived(&thread_id);
        match &derived[1] {
            DerivedMessage::Assistant { tool_calls, .. } => {
                assert_eq!(tool_calls.len(), 1);
                assert_eq!(tool_calls[0].call_id, "c1");
                assert_eq!(tool_calls[0].name, "write");
            }
            other => panic!("expected derived tool call from tool/call, got {other:?}"),
        }
        match &derived[2] {
            DerivedMessage::Tool {
                tool_call_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_call_id, "c1");
                assert!(!*is_error);
            }
            other => panic!("expected tool result, got {other:?}"),
        }
        match &derived[3] {
            DerivedMessage::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content, "wrote it");
                assert!(tool_calls.is_empty());
            }
            other => panic!("expected final text, got {other:?}"),
        }

        let projected = theseus_core::project_items(&events);
        let notified: Vec<ThreadItem> = turn
            .outgoing
            .iter()
            .filter_map(|o| match o {
                Outgoing::Notification(n) if n.method == "item/completed" => {
                    serde_json::from_value(n.params["item"].clone()).ok()
                }
                _ => None,
            })
            .collect();
        assert_eq!(notified, projected);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn tool_loop_read_edit_bash_and_does_not_cross_steps() {
        let dir = temp_workspace("four");
        fs::write(dir.join("note.txt"), "hello world\n").unwrap();
        let seam = ScriptedSeam::rounds([
            ScriptedRound::ToolCalls {
                text: String::new(),
                calls: vec![
                    call_read("note.txt"),
                    ToolCallRequest {
                        id: "c_edit".into(),
                        name: "edit".into(),
                        arguments: r#"{"path":"note.txt","oldString":"world","newString":"peng"}"#
                            .into(),
                    },
                    ToolCallRequest {
                        id: "c_bash".into(),
                        name: "bash".into(),
                        arguments: r#"{"command":"echo from-bash"}"#.into(),
                    },
                ],
            },
            ScriptedRound::Text(vec!["done".into()]),
        ]);
        let mut server = test_server(seam);
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "use tools" }]
            }),
        ));

        assert_eq!(
            fs::read_to_string(dir.join("note.txt")).unwrap(),
            "hello peng\n"
        );
        let derived = server.derived(&thread_id);
        let calls: Vec<&str> = derived
            .iter()
            .filter_map(|m| match m {
                DerivedMessage::Assistant { tool_calls, .. } => Some(tool_calls.as_slice()),
                _ => None,
            })
            .flatten()
            .map(|c| c.call_id.as_str())
            .collect();
        assert_eq!(calls, vec!["c_read", "c_edit", "c_bash"]);
        let step1 = derived.iter().find_map(|m| match m {
            DerivedMessage::Assistant { tool_calls, .. } if !tool_calls.is_empty() => {
                Some(tool_calls.len())
            }
            _ => None,
        });
        assert_eq!(step1, Some(3), "same-step calls merge onto one assistant");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn path_escape_is_tool_error_and_does_not_abort_turn() {
        let dir = temp_workspace("esc");
        let seam = ScriptedSeam::rounds([
            ScriptedRound::ToolCalls {
                text: String::new(),
                calls: vec![ToolCallRequest {
                    id: "c_esc".into(),
                    name: "read".into(),
                    arguments: r#"{"path":"../outside.txt"}"#.into(),
                }],
            },
            ScriptedRound::Text(vec!["blocked".into()]),
        ]);
        let mut server = test_server(seam);
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "read outside" }]
            }),
        ));
        assert_eq!(
            session_event_types(&turn),
            vec![
                "turn/start",
                "user/message",
                "step/start",
                "tool/call",
                "tool/result",
                "step/end",
                "step/start",
                "assistant/message",
                "step/end",
                "turn/end",
            ]
        );
        let result = turn.outgoing.iter().find_map(|o| match o {
            Outgoing::Notification(n) if n.method == "session/event" => {
                if n.params["event"]["type"] == "tool/result" {
                    Some((
                        n.params["event"]["data"]["isError"]
                            .as_bool()
                            .unwrap_or(false),
                        n.params["event"]["data"]["content"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                    ))
                } else {
                    None
                }
            }
            _ => None,
        });
        let (is_error, content) = result.expect("tool/result");
        assert!(is_error);
        assert!(content.contains("escapes workspace"), "{content}");
        let end = turn.outgoing.iter().find_map(|o| match o {
            Outgoing::Notification(n) if n.method == "session/event" => {
                if n.params["event"]["type"] == "turn/end" {
                    n.params["event"]["data"]["reason"]["kind"]
                        .as_str()
                        .map(str::to_string)
                } else {
                    None
                }
            }
            _ => None,
        });
        assert_eq!(end.as_deref(), Some("completed"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn max_steps_aborts_cleanly_without_half_cut() {
        let dir = temp_workspace("cap");
        fs::write(dir.join("note.txt"), "x\n").unwrap();
        let seam = ScriptedSeam::rounds([
            ScriptedRound::ToolCalls {
                text: String::new(),
                calls: vec![call_read("note.txt")],
            },
            ScriptedRound::ToolCalls {
                text: String::new(),
                calls: vec![ToolCallRequest {
                    id: "c_read2".into(),
                    name: "read".into(),
                    arguments: r#"{"path":"note.txt"}"#.into(),
                }],
            },
            ScriptedRound::Text(vec!["should not run".into()]),
        ]);
        let mut server = test_seams(seam, LocalToolSeam, 2);
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "loop" }]
            }),
        ));

        let types = server.event_types(&thread_id);
        assert_eq!(
            types,
            vec![
                "thread/meta",
                "turn/start",
                "user/message",
                "step/start",
                "tool/call",
                "tool/result",
                "step/end",
                "step/start",
                "tool/call",
                "tool/result",
                "step/end",
                "turn/end",
            ]
        );
        assert!(!types.iter().any(|t| t == "assistant/message"));
        assert_eq!(types.iter().filter(|t| **t == "step/start").count(), 2);

        let end = turn.outgoing.iter().find_map(|o| match o {
            Outgoing::Notification(n) if n.method == "session/event" => {
                if n.params["event"]["type"] == "turn/end" {
                    Some((
                        n.params["event"]["data"]["reason"]["kind"]
                            .as_str()?
                            .to_string(),
                        n.params["event"]["data"]["reason"]["reason"]
                            .as_str()
                            .map(str::to_string),
                    ))
                } else {
                    None
                }
            }
            _ => None,
        });
        assert_eq!(
            end,
            Some(("aborted".into(), Some("max steps reached (2)".into())))
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn bash_deadline_is_tool_error_not_half_cut() {
        let dir = temp_workspace("bash-dl");
        let seam = ScriptedSeam::rounds([
            ScriptedRound::ToolCalls {
                text: String::new(),
                calls: vec![ToolCallRequest {
                    id: "c_spin".into(),
                    name: "bash".into(),
                    arguments: r#"{"command":"while true; do true; done","timeout":0.2}"#.into(),
                }],
            },
            ScriptedRound::Text(vec!["deadline hit, moving on".into()]),
        ]);
        let mut server = test_server(seam);
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "sleep" }]
            }),
        ));
        let types = session_event_types(&turn);
        assert!(types.contains(&"tool/result".to_string()));
        assert!(types.contains(&"assistant/message".to_string()));
        assert!(types.contains(&"turn/end".to_string()));

        let result = turn.outgoing.iter().find_map(|o| match o {
            Outgoing::Notification(n) if n.method == "session/event" => {
                if n.params["event"]["type"] == "tool/result" {
                    Some((
                        n.params["event"]["data"]["isError"]
                            .as_bool()
                            .unwrap_or(false),
                        n.params["event"]["data"]["content"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                    ))
                } else {
                    None
                }
            }
            _ => None,
        });
        let (is_error, content) = result.expect("tool/result");
        assert!(is_error);
        assert!(content.contains("timed out"), "{content}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn thread_start_writes_jsonl_and_reload_matches() {
        let dir = temp_sessions();
        let mut server =
            AppServer::with_llm(ScriptedSeam::ok(["hi"])).with_sessions_dir(dir.clone());
        let thread_id = start_thread(&mut server);
        let path = server.thread_path(&thread_id).expect("Thread.path");
        assert!(path.ends_with(&format!("{thread_id}.jsonl")), "{path}");
        assert!(std::path::Path::new(&path).is_file());
        assert_eq!(server.threads[&thread_id].thread.ephemeral, false);

        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }]
            }),
        ));
        assert!(session_event_types(&turn).contains(&"turn/end".to_string()));

        let live = server.events(&thread_id);
        let derived = server.derived(&thread_id);
        let items = server.threads[&thread_id].session.project_items();

        let reloaded = Session::load_jsonl(&path).unwrap();
        assert_eq!(reloaded.events(), live.as_slice());
        assert_eq!(reloaded.derive_messages(), derived);
        assert_eq!(reloaded.project_items(), items);

        let fresh = Session::load_thread(&dir, &thread_id, DEFAULT_MAX_STEPS_PER_TURN).unwrap();
        assert_eq!(fresh.events(), live.as_slice());
        assert_eq!(fresh.derive_messages(), derived);

        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("assistant/chunk"));
        assert!(!text.contains("\"role\":\"assistant\""));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn persist_failure_fails_turn_and_does_not_keep_event_only_in_memory() {
        let dir = temp_sessions();
        let mut server =
            AppServer::with_llm(ScriptedSeam::ok(["hi"])).with_sessions_dir(dir.clone());
        let thread_id = start_thread(&mut server);
        let path = PathBuf::from(server.thread_path(&thread_id).unwrap());
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        let before = server.event_types(&thread_id);
        assert_eq!(before, vec!["thread/meta"]);

        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hello" }]
            }),
        ));
        match &turn.outgoing[0] {
            Outgoing::Response(r) => {
                let msg = r.error.as_ref().expect("rpc error").message.clone();
                assert!(
                    msg.contains("session log")
                        || msg.contains("io")
                        || msg.contains("Is a directory"),
                    "{msg}"
                );
            }
            other => panic!("expected RPC error, got {other:?}"),
        }
        assert_eq!(
            server.event_types(&thread_id),
            vec!["thread/meta"],
            "failed append must not live only in memory"
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn thread_from_result(result: &HandleResult) -> Thread {
        result
            .outgoing
            .iter()
            .find_map(|o| match o {
                Outgoing::Response(r) => {
                    serde_json::from_value(r.result.as_ref()?.get("thread")?.clone()).ok()
                }
                _ => None,
            })
            .expect("thread in result")
    }

    #[test]
    fn resume_on_new_server_matches_events_and_derive_then_only_appends() {
        let dir = temp_sessions();
        let mut first =
            AppServer::with_llm(ScriptedSeam::ok(["hello"])).with_sessions_dir(dir.clone());
        let thread_id = start_thread(&mut first);
        first.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "hi" }]
            }),
        ));
        let live = first.events(&thread_id);
        let derived = first.derived(&thread_id);
        let path = first.thread_path(&thread_id).unwrap();
        drop(first);

        let mut second =
            AppServer::with_llm(ScriptedSeam::ok(["again"])).with_sessions_dir(dir.clone());
        second.handle_request(req(
            0,
            "initialize",
            json!({ "clientInfo": { "name": "test", "version": "0.1.0" } }),
        ));

        let listed = second.handle_request(req(1, "thread/list", json!({})));
        let ids: Vec<String> = listed
            .outgoing
            .iter()
            .find_map(|o| match o {
                Outgoing::Response(r) => Some(
                    r.result.as_ref()?["threads"]
                        .as_array()?
                        .iter()
                        .filter_map(|t| t["id"].as_str().map(str::to_string))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap();
        assert_eq!(ids, vec![thread_id.clone()]);
        let row = listed.outgoing.iter().find_map(|o| match o {
            Outgoing::Response(r) => r.result.as_ref()?["threads"].as_array()?.first().cloned(),
            _ => None,
        });
        let row = row.expect("list row");
        assert_eq!(row["id"], thread_id);
        assert_eq!(row["path"], path);
        assert!(row.get("updatedAt").and_then(|v| v.as_i64()).is_some());
        assert_eq!(row["preview"], "hello");
        assert!(row.get("pinned").is_none());
        assert!(row.get("folder").is_none());

        let resumed =
            second.handle_request(req(2, "thread/resume", json!({ "threadId": thread_id })));
        let thread = thread_from_result(&resumed);
        assert_eq!(thread.id, thread_id);
        assert_eq!(thread.path.as_deref(), Some(path.as_str()));
        assert_eq!(thread.preview, "hello");
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].items, theseus_core::project_items(&live));
        assert_eq!(second.events(&thread_id), live);
        assert_eq!(second.derived(&thread_id), derived);

        second.handle_request(req(
            3,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "next" }]
            }),
        ));
        let after = second.events(&thread_id);
        assert_eq!(&after[..live.len()], live.as_slice());
        assert!(after.len() > live.len());
        assert_eq!(
            after[live.len()].type_name(),
            "turn/start",
            "resume must append, not rewrite history"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn list_skips_corrupt_jsonl_and_keeps_good_rows() {
        let dir = temp_sessions();
        let mut server =
            AppServer::with_llm(ScriptedSeam::ok(["ok"])).with_sessions_dir(dir.clone());
        let good = start_thread(&mut server);
        fs::write(dir.join("thr_bad.jsonl"), "{not json\n").unwrap();
        fs::write(dir.join("notes.txt"), "ignore").unwrap();

        let listed = server.handle_request(req(3, "thread/list", json!({ "limit": 20 })));
        let rows: Vec<Value> = listed
            .outgoing
            .iter()
            .find_map(|o| match o {
                Outgoing::Response(r) => r.result.as_ref()?["threads"].as_array().cloned(),
                _ => None,
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], good);
        assert!(rows.iter().all(|r| r["id"] != "thr_bad"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resume_rejects_path_outside_sessions_dir() {
        let mut server = test_server(ScriptedSeam::ok(["x"]));
        start_thread(&mut server);
        let out = server.handle_request(req(3, "thread/resume", json!({ "path": "/etc/passwd" })));
        match &out.outgoing[0] {
            Outgoing::Response(r) => {
                let msg = r.error.as_ref().unwrap().message.clone();
                assert!(
                    msg.contains("escapes") || msg.contains("not found") || msg.contains("jsonl"),
                    "{msg}"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    fn assert_assistant_json_omits_tool_calls(events: &[SessionEvent]) {
        for event in events {
            if matches!(event.data, EventData::AssistantMessage(_)) {
                let json = serde_json::to_value(event).unwrap();
                assert!(
                    json["data"].get("tool_calls").is_none()
                        && json["data"]["message"].get("tool_calls").is_none(),
                    "assistant/message must not write tool_calls: {json}"
                );
            }
        }
    }

    fn notify_named<'a>(result: &'a HandleResult, method: &str) -> Vec<&'a JsonRpcNotification> {
        result
            .outgoing
            .iter()
            .filter_map(|o| match o {
                Outgoing::Notification(n) if n.method == method => Some(n),
                _ => None,
            })
            .collect()
    }

    fn last_tool_result(server: &AppServer, thread_id: &str) -> Option<(bool, String)> {
        server
            .events(thread_id)
            .into_iter()
            .rev()
            .find_map(|e| match e.data {
                EventData::ToolResult(r) => Some((r.is_error, r.content)),
                _ => None,
            })
    }

    fn write_then_text(path: &str, content: &str) -> ScriptedSeam {
        ScriptedSeam::rounds([
            ScriptedRound::ToolCalls {
                text: String::new(),
                calls: vec![ToolCallRequest {
                    id: "c_write".into(),
                    name: "write".into(),
                    arguments: format!(r#"{{"path":"{path}","content":"{content}"}}"#),
                }],
            },
            ScriptedRound::Text(vec!["done".into()]),
        ])
    }

    #[test]
    fn approve_mode_write_does_not_touch_disk_until_granted() {
        let dir = temp_workspace("gate-write");
        let target = dir.join("out.txt");
        let mut server = approve_server(
            write_then_text("out.txt", "secret"),
            Duration::from_secs(60),
        );
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        let parked = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "write it" }]
            }),
        ));
        assert!(!target.exists(), "must not write before approve");
        assert_eq!(
            session_event_types(&parked),
            vec!["turn/start", "user/message", "step/start", "tool/call",]
        );
        let reqs = notify_named(&parked, "item/tool/approval/request");
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].params["tool"], "write");
        assert_eq!(reqs[0].params["callId"], "c_write");
        assert!(reqs[0].params["summary"]
            .as_str()
            .unwrap()
            .contains("out.txt"));
        assert!(server.turn_is_open(&thread_id));
        assert!(server.step_is_open(&thread_id));

        let granted = server.handle_request(req(
            3,
            "tool/approve",
            json!({ "threadId": thread_id, "callId": "c_write" }),
        ));
        assert_eq!(fs::read_to_string(&target).unwrap(), "secret");
        assert_eq!(
            notify_named(&granted, "item/tool/approval/resolved")[0].params["decision"],
            "approved"
        );
        assert!(session_event_types(&granted).contains(&"tool/result".to_string()));
        assert!(session_event_types(&granted).contains(&"turn/end".to_string()));
        assert!(!server.turn_is_open(&thread_id));
        assert!(!server.step_is_open(&thread_id));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn approve_mode_reject_is_error_and_does_not_write() {
        let dir = temp_workspace("gate-reject");
        let target = dir.join("out.txt");
        let mut server =
            approve_server(write_then_text("out.txt", "nope"), Duration::from_secs(60));
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "write it" }]
            }),
        ));
        let rejected = server.handle_request(req(
            3,
            "tool/reject",
            json!({ "threadId": thread_id, "callId": "c_write" }),
        ));
        assert!(!target.exists());
        let (is_error, content) = last_tool_result(&server, &thread_id).expect("result");
        assert!(is_error, "{content}");
        assert!(content.contains("rejected"), "{content}");
        assert!(session_event_types(&rejected).contains(&"tool/result".to_string()));
        assert!(session_event_types(&rejected).contains(&"turn/end".to_string()));
        assert!(!server.turn_is_open(&thread_id));
        assert!(!server.step_is_open(&thread_id));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn approve_mode_timeout_is_reject_and_closes_turn() {
        let dir = temp_workspace("gate-timeout");
        let target = dir.join("out.txt");
        let mut server = approve_server(write_then_text("out.txt", "late"), Duration::ZERO);
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "write it" }]
            }),
        ));
        assert!(!target.exists());
        assert_eq!(server.pending_call_id().as_deref(), Some("c_write"));

        let listed = server.handle_request(req(3, "thread/list", json!({})));
        assert!(!target.exists(), "timeout must not execute write");
        assert!(server.pending_call_id().is_none());
        let (is_error, content) = last_tool_result(&server, &thread_id).expect("result");
        assert!(is_error, "{content}");
        assert!(content.contains("timed out"), "{content}");
        assert!(session_event_types(&listed).contains(&"tool/result".to_string()));
        assert!(session_event_types(&listed).contains(&"turn/end".to_string()));
        assert!(
            !server.turn_is_open(&thread_id),
            "timeout must not leave turn open"
        );
        assert!(
            !server.step_is_open(&thread_id),
            "timeout must not leave step open"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn approve_mode_read_never_waits() {
        let dir = temp_workspace("gate-read");
        fs::write(dir.join("note.txt"), "hello\n").unwrap();
        let mut server = approve_server(
            ScriptedSeam::rounds([
                ScriptedRound::ToolCalls {
                    text: String::new(),
                    calls: vec![call_read("note.txt")],
                },
                ScriptedRound::Text(vec!["saw it".into()]),
            ]),
            Duration::from_secs(60),
        );
        let thread_id = start_thread_with(
            &mut server,
            json!({ "model": "gpt-4o-mini", "cwd": dir.to_string_lossy() }),
        );
        let turn = server.handle_request(req(
            2,
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": "read it" }]
            }),
        ));
        assert!(notify_named(&turn, "item/tool/approval/request").is_empty());
        assert!(session_event_types(&turn).contains(&"tool/result".to_string()));
        assert!(session_event_types(&turn).contains(&"turn/end".to_string()));
        let (is_error, content) = last_tool_result(&server, &thread_id).unwrap();
        assert!(!is_error, "{content}");
        assert!(content.contains("hello"), "{content}");
        let _ = fs::remove_dir_all(dir);
    }
}
