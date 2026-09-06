use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use pi_protocol::{
    DerivedMessage, EventData, SessionEvent, StepEnd, StepStart, ThreadItem, ThreadMeta, TurnEnd,
    TurnEndReason, TurnStart,
};
use thiserror::Error;

use crate::derive::derive_messages;
use crate::fence::{Fence, FenceError, DEFAULT_MAX_STEPS_PER_TURN};
use crate::project::{project_items, project_items_for_turn};
use crate::store::{session_log_path, PersistError, SessionLog};

/// Fence violation or a failed JSONL append/load.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionError {
    #[error(transparent)]
    Fence(#[from] FenceError),
    #[error(transparent)]
    Persist(#[from] PersistError),
}

/// Append-only session. The event log is the only model-visible state.
///
/// When a [`SessionLog`] is attached, every successful append is fsynced to
/// that JSONL first; a persist failure leaves memory unchanged.
#[derive(Debug, Clone)]
pub struct Session {
    id: String,
    events: Vec<SessionEvent>,
    fence: Fence,
    log: Option<SessionLog>,
}

impl Session {
    /// Empty session. Call [`Session::append`] with `thread/meta` first (or use [`Session::create`]).
    pub fn new(id: impl Into<String>) -> Self {
        Self::new_with_max_steps(id, DEFAULT_MAX_STEPS_PER_TURN)
    }

    /// Empty session with a custom per-turn step cap (minimum 1).
    pub fn new_with_max_steps(id: impl Into<String>, max_steps: u32) -> Self {
        Self {
            id: id.into(),
            events: Vec::new(),
            fence: Fence::new(max_steps),
            log: None,
        }
    }

    /// Create a session and append `thread/meta` as seq 0.
    pub fn create(meta: ThreadMeta) -> Result<Self, SessionError> {
        Self::create_with_max_steps(meta, DEFAULT_MAX_STEPS_PER_TURN)
    }

    /// [`Session::create`] with a custom per-turn step cap.
    pub fn create_with_max_steps(meta: ThreadMeta, max_steps: u32) -> Result<Self, SessionError> {
        let mut session = Self::new_with_max_steps(meta.thread_id.clone(), max_steps);
        session.append(EventData::ThreadMeta(meta))?;
        Ok(session)
    }

    /// Replay a borrowed seed (resume / fork). Events must already be fenced.
    pub fn from_events(
        id: impl Into<String>,
        events: Vec<SessionEvent>,
    ) -> Result<Self, FenceError> {
        Self::from_events_with_max(id, events, DEFAULT_MAX_STEPS_PER_TURN)
    }

    /// [`Session::from_events`] with a custom per-turn step cap.
    pub fn from_events_with_max(
        id: impl Into<String>,
        events: Vec<SessionEvent>,
        max_steps: u32,
    ) -> Result<Self, FenceError> {
        let mut session = Self::new_with_max_steps(id, max_steps);
        for event in events {
            session.fence.accept(&event.data)?;
            session.events.push(event);
        }
        Ok(session)
    }

    /// Hard cap on `step/start` in the current session (default 20).
    pub fn max_steps_per_turn(&self) -> u32 {
        self.fence.max_steps_per_turn()
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    /// Next event sequence number — always the log length.
    pub fn seq(&self) -> u64 {
        self.events.len() as u64
    }

    pub fn open_turn(&self) -> Option<u32> {
        self.fence.open_turn()
    }

    pub fn open_step(&self) -> Option<(u32, u32)> {
        self.fence.open_step()
    }

    pub fn next_turn(&self) -> u32 {
        self.fence.next_turn()
    }

    pub fn next_step(&self) -> u32 {
        self.fence.next_step()
    }

    /// Create a session that claims `{sessions_dir}/{thread_id}.jsonl` and writes `thread/meta`.
    pub fn create_persisted(
        meta: ThreadMeta,
        max_steps: u32,
        sessions_dir: impl AsRef<Path>,
    ) -> Result<Self, SessionError> {
        let thread_id = meta.thread_id.clone();
        let log = SessionLog::create(sessions_dir.as_ref(), &thread_id)?;
        let path = log.path().to_path_buf();
        let mut session = Self::new_with_max_steps(thread_id, max_steps);
        session.log = Some(log);
        match session.append(EventData::ThreadMeta(meta)) {
            Ok(_) => Ok(session),
            Err(err) => {
                let _ = std::fs::remove_file(path);
                Err(err)
            }
        }
    }

    /// Replay a JSONL file into a new Session (same fence / derive / project rules).
    pub fn load_jsonl(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        Self::load_jsonl_with_max(path, DEFAULT_MAX_STEPS_PER_TURN)
    }

    /// [`Session::load_jsonl`] with a custom per-turn step cap.
    pub fn load_jsonl_with_max(
        path: impl AsRef<Path>,
        max_steps: u32,
    ) -> Result<Self, SessionError> {
        let (log, events) = SessionLog::open(path)?;
        let id = match events.first() {
            Some(SessionEvent {
                data: EventData::ThreadMeta(meta),
                ..
            }) => meta.thread_id.clone(),
            Some(_) => return Err(SessionError::Fence(FenceError::MetaMustBeFirst)),
            None => return Err(PersistError::Empty.into()),
        };
        if let Some(stem) = log
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
        {
            if stem != id {
                return Err(PersistError::ThreadIdMismatch {
                    file: stem.to_string(),
                    meta: id,
                }
                .into());
            }
        }
        let mut session = Self::from_events_with_max(id, events, max_steps)?;
        session.log = Some(log);
        Ok(session)
    }

    /// Load `{sessions_dir}/{thread_id}.jsonl`.
    pub fn load_thread(
        sessions_dir: impl AsRef<Path>,
        thread_id: &str,
        max_steps: u32,
    ) -> Result<Self, SessionError> {
        let path = session_log_path(sessions_dir, thread_id)?;
        Self::load_jsonl_with_max(path, max_steps)
    }

    /// Absolute JSONL path when this session is persisted.
    pub fn log_path(&self) -> Option<&Path> {
        self.log.as_ref().map(SessionLog::path)
    }

    /// Append one typed event. `seq` / `time` are assigned here.
    ///
    /// If a store is attached, the line is fsynced **before** memory/fence commit.
    /// Persist failure does not apply the event.
    pub fn append(&mut self, data: EventData) -> Result<SessionEvent, SessionError> {
        self.fence.check(&data)?;
        let event = SessionEvent {
            seq: self.seq(),
            time: now_ms(),
            data,
        };
        if let Some(log) = &self.log {
            log.append(&event)?;
        }
        self.fence.apply(&event.data);
        self.events.push(event.clone());
        Ok(event)
    }

    pub fn start_turn(&mut self) -> Result<SessionEvent, SessionError> {
        let turn = self.next_turn();
        self.append(EventData::TurnStart(TurnStart { turn }))
    }

    pub fn end_turn(&mut self, reason: TurnEndReason) -> Result<SessionEvent, SessionError> {
        let turn = self.open_turn().ok_or(FenceError::NoOpenTurn)?;
        self.append(EventData::TurnEnd(TurnEnd { turn, reason }))
    }

    pub fn start_step(&mut self) -> Result<SessionEvent, SessionError> {
        let turn = self.open_turn().ok_or(FenceError::NoOpenTurn)?;
        let step = self.next_step();
        self.append(EventData::StepStart(StepStart { turn, step }))
    }

    pub fn end_step(&mut self) -> Result<SessionEvent, SessionError> {
        let (turn, step) = self.open_step().ok_or(FenceError::NoOpenStep)?;
        self.append(EventData::StepEnd(StepEnd { turn, step }))
    }

    /// Model-visible history derived from appended events only.
    pub fn derive_messages(&self) -> Vec<DerivedMessage> {
        derive_messages(&self.events)
    }

    /// Codex Items derived from the same log as [`Self::derive_messages`].
    pub fn project_items(&self) -> Vec<ThreadItem> {
        project_items(&self.events)
    }

    /// Items for one turn, derived from the log (not assembled by the server).
    pub fn project_items_for_turn(&self, turn: u32) -> Vec<ThreadItem> {
        project_items_for_turn(&self.events, turn)
    }

    /// Short preview from the last user or assistant text in this log.
    pub fn preview(&self) -> String {
        preview_from_events(&self.events, 80)
    }

    /// Copy the inclusive prefix `events[0..=through_seq]` into a new session.
    ///
    /// The prefix must end outside an open turn (deepseek-harness `fork` policy).
    pub fn fork(&self, child_id: impl Into<String>, through_seq: u64) -> Result<Self, FenceError> {
        if through_seq >= self.seq() {
            return Err(FenceError::SeqOutOfRange(through_seq));
        }
        let prefix: Vec<SessionEvent> = self.events[..=through_seq as usize].to_vec();
        let child = Self::from_events_with_max(child_id, prefix, self.max_steps_per_turn())?;
        if child.open_turn().is_some() {
            return Err(FenceError::ForkInsideTurn(through_seq));
        }
        Ok(child)
    }
}

/// Last non-empty `user/message` or `assistant/message` text, truncated.
pub fn preview_from_events(events: &[SessionEvent], max_chars: usize) -> String {
    let mut text = "";
    for ev in events.iter().rev() {
        match &ev.data {
            EventData::AssistantMessage(m) if !m.message.content.is_empty() => {
                text = m.message.content.as_str();
                break;
            }
            EventData::UserMessage(m) if !m.content.is_empty() => {
                text = m.content.as_str();
                break;
            }
            _ => {}
        }
    }
    text.chars().take(max_chars).collect()
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
    use pi_protocol::{
        AssistantChunk, AssistantMessageBody, AssistantMessageEvent, ToolCallEvent,
        ToolResultEvent, UserMessageEvent,
    };
    use std::path::PathBuf;

    fn meta(id: &str) -> ThreadMeta {
        ThreadMeta {
            thread_id: id.into(),
            model: Some("stub".into()),
            cwd: None,
            title: None,
            created_at: 0,
            parent_thread_id: None,
        }
    }

    fn user(turn: u32, text: &str) -> EventData {
        EventData::UserMessage(UserMessageEvent {
            turn,
            id: format!("item_u_{turn}"),
            content: text.into(),
            source: Some("user".into()),
        })
    }

    fn assistant(turn: u32, step: u32, text: &str) -> EventData {
        EventData::AssistantMessage(AssistantMessageEvent {
            turn,
            step,
            message: AssistantMessageBody::text(format!("item_a_{turn}_{step}"), text),
            usage: None,
            interrupted: None,
        })
    }

    #[test]
    fn append_happy_path_and_derive() {
        let mut s = Session::create(meta("thr_1")).unwrap();
        s.start_turn().unwrap();
        s.append(user(1, "hello")).unwrap();
        s.start_step().unwrap();
        s.append(assistant(1, 1, "world")).unwrap();
        s.end_step().unwrap();
        s.end_turn(TurnEndReason::Completed).unwrap();

        assert_eq!(s.seq(), 7);
        assert_eq!(
            s.events().iter().map(|e| e.type_name()).collect::<Vec<_>>(),
            vec![
                "thread/meta",
                "turn/start",
                "user/message",
                "step/start",
                "assistant/message",
                "step/end",
                "turn/end",
            ]
        );
        assert_eq!(
            s.derive_messages(),
            vec![
                DerivedMessage::User {
                    content: "hello".into()
                },
                DerivedMessage::Assistant {
                    content: "world".into(),
                    tool_calls: vec![],
                },
            ]
        );
    }

    #[test]
    fn fence_rejects_unbalanced_and_chunks() {
        let mut s = Session::create(meta("thr_1")).unwrap();
        assert_eq!(
            s.end_turn(TurnEndReason::Completed).unwrap_err(),
            SessionError::Fence(FenceError::NoOpenTurn)
        );
        s.start_turn().unwrap();
        assert_eq!(
            s.start_turn().unwrap_err(),
            SessionError::Fence(FenceError::TurnAlreadyOpen(1))
        );
        assert_eq!(
            s.append(assistant(1, 1, "nope")).unwrap_err(),
            SessionError::Fence(FenceError::OutsideStep)
        );
        s.start_step().unwrap();
        assert_eq!(
            s.append(EventData::AssistantChunk(AssistantChunk {
                turn: 1,
                step: 1,
                text: "x".into(),
            }))
            .unwrap_err(),
            SessionError::Fence(FenceError::ChunkNotInHistory)
        );
        assert_eq!(
            s.end_turn(TurnEndReason::Completed).unwrap_err(),
            SessionError::Fence(FenceError::CloseTurnWithOpenStep { turn: 1, step: 1 })
        );
    }

    #[test]
    fn tool_pairing_and_duplicate_call() {
        let mut s = Session::create(meta("thr_1")).unwrap();
        s.start_turn().unwrap();
        s.append(user(1, "use echo")).unwrap();
        s.start_step().unwrap();
        s.append(EventData::ToolCall(ToolCallEvent {
            turn: 1,
            step: 1,
            call_id: "c1".into(),
            name: "echo".into(),
            arguments: "{}".into(),
        }))
        .unwrap();
        assert_eq!(
            s.append(EventData::ToolCall(ToolCallEvent {
                turn: 1,
                step: 1,
                call_id: "c1".into(),
                name: "echo".into(),
                arguments: "{}".into(),
            }))
            .unwrap_err(),
            SessionError::Fence(FenceError::DuplicateToolCall("c1".into()))
        );
        assert_eq!(
            s.append(EventData::ToolResult(ToolResultEvent {
                turn: 1,
                step: 1,
                call_id: "missing".into(),
                content: "x".into(),
                is_error: false,
            }))
            .unwrap_err(),
            SessionError::Fence(FenceError::UnknownToolCall("missing".into()))
        );
        s.append(EventData::ToolResult(ToolResultEvent {
            turn: 1,
            step: 1,
            call_id: "c1".into(),
            content: "ok".into(),
            is_error: false,
        }))
        .unwrap();
        s.end_step().unwrap();
        s.end_turn(TurnEndReason::Completed).unwrap();

        let msgs = s.derive_messages();
        assert!(matches!(msgs[0], DerivedMessage::User { .. }));
        assert!(matches!(
            &msgs[1],
            DerivedMessage::Assistant { tool_calls, .. } if tool_calls.len() == 1
        ));
        assert!(matches!(
            &msgs[2],
            DerivedMessage::Tool { tool_call_id, .. } if tool_call_id == "c1"
        ));
    }

    #[test]
    fn fork_copies_prefix_and_rejects_open_turn() {
        let mut s = Session::create(meta("thr_1")).unwrap();
        s.start_turn().unwrap();
        s.append(user(1, "hello")).unwrap();
        s.start_step().unwrap();
        s.append(assistant(1, 1, "world")).unwrap();
        s.end_step().unwrap();
        let turn_end = s.end_turn(TurnEndReason::Completed).unwrap();

        let child = s.fork("thr_2", turn_end.seq).unwrap();
        assert_eq!(child.seq(), s.seq());
        assert_eq!(child.events(), s.events());
        assert_eq!(child.derive_messages(), s.derive_messages());

        // Child is independent: further appends do not grow the parent.
        let mut child = child;
        child.start_turn().unwrap();
        assert_eq!(child.seq(), s.seq() + 1);
        assert_eq!(s.seq(), turn_end.seq + 1);

        let mut open = Session::create(meta("thr_3")).unwrap();
        open.start_turn().unwrap();
        let err = open.fork("thr_4", 1).unwrap_err();
        assert_eq!(err, FenceError::ForkInsideTurn(1));
    }

    #[test]
    fn default_max_steps_is_twenty() {
        let s = Session::create(meta("thr_1")).unwrap();
        assert_eq!(s.max_steps_per_turn(), DEFAULT_MAX_STEPS_PER_TURN);
        assert_eq!(DEFAULT_MAX_STEPS_PER_TURN, 20);
    }

    #[test]
    fn max_steps_rejects_next_start_and_allows_clean_turn_end() {
        let mut s = Session::create_with_max_steps(meta("thr_1"), 2).unwrap();
        s.start_turn().unwrap();
        s.append(user(1, "loop")).unwrap();
        s.start_step().unwrap();
        s.append(assistant(1, 1, "a")).unwrap();
        s.end_step().unwrap();
        s.start_step().unwrap();
        s.append(assistant(1, 2, "b")).unwrap();
        s.end_step().unwrap();
        assert_eq!(
            s.start_step().unwrap_err(),
            SessionError::Fence(FenceError::MaxStepsReached {
                turn: 1,
                max: 2,
                attempted: 3
            })
        );
        assert!(
            s.open_step().is_none(),
            "cap must not leave a half-open step"
        );
        s.end_turn(TurnEndReason::Aborted {
            reason: Some("max steps reached (2)".into()),
        })
        .unwrap();
        assert!(s.open_turn().is_none());
    }

    #[test]
    fn derive_reads_only_appended_events() {
        let mut s = Session::create(meta("thr_1")).unwrap();
        assert!(s.derive_messages().is_empty());
        s.start_turn().unwrap();
        assert!(
            s.derive_messages().is_empty(),
            "turn/start is not model-visible"
        );
        s.append(user(1, "only this")).unwrap();
        assert_eq!(
            s.derive_messages(),
            vec![DerivedMessage::User {
                content: "only this".into()
            }]
        );
    }

    fn temp_sessions(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pi-core-sess-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn persist_append_reload_matches_events_and_derive() {
        let dir = temp_sessions("roundtrip");
        let mut s = Session::create_persisted(meta("thr_1"), DEFAULT_MAX_STEPS_PER_TURN, &dir)
            .unwrap();
        s.start_turn().unwrap();
        s.append(user(1, "hello")).unwrap();
        s.start_step().unwrap();
        s.append(assistant(1, 1, "world")).unwrap();
        s.end_step().unwrap();
        s.end_turn(TurnEndReason::Completed).unwrap();

        let path = s.log_path().unwrap().to_path_buf();
        assert_eq!(path, dir.join("thr_1.jsonl"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("assistant/chunk"));
        assert!(!text.contains("\"role\""));

        let reloaded = Session::load_thread(&dir, "thr_1", DEFAULT_MAX_STEPS_PER_TURN).unwrap();
        assert_eq!(reloaded.id(), s.id());
        assert_eq!(reloaded.events(), s.events());
        assert_eq!(reloaded.derive_messages(), s.derive_messages());
        assert_eq!(reloaded.project_items(), s.project_items());
        assert_eq!(reloaded.open_turn(), None);

        let from_path = Session::load_jsonl(&path).unwrap();
        assert_eq!(from_path.events(), s.events());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn persist_failure_does_not_keep_event_in_memory() {
        let dir = temp_sessions("fail");
        let mut s = Session::create_persisted(meta("thr_1"), DEFAULT_MAX_STEPS_PER_TURN, &dir)
            .unwrap();
        let path = s.log_path().unwrap().to_path_buf();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        let before = s.events().to_vec();
        let err = s.start_turn().unwrap_err();
        assert!(matches!(err, SessionError::Persist(_)), "{err:?}");
        assert_eq!(s.events(), before.as_slice());
        assert!(s.open_turn().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn load_rejects_bad_and_truncated_jsonl() {
        let dir = temp_sessions("badload");
        let path = dir.join("thr_1.jsonl");
        std::fs::write(&path, "{not json\n").unwrap();
        assert!(matches!(
            Session::load_jsonl(&path).unwrap_err(),
            SessionError::Persist(PersistError::InvalidLine { line: 1, .. })
                | SessionError::Persist(PersistError::Truncated { line: 1 })
        ));

        let meta_line = serde_json::to_string(&SessionEvent {
            seq: 0,
            time: 1,
            data: EventData::ThreadMeta(meta("thr_1")),
        })
        .unwrap();
        std::fs::write(&path, format!("{meta_line}\n{{\"type\":\"turn/start\"")).unwrap();
        assert!(matches!(
            Session::load_jsonl(&path).unwrap_err(),
            SessionError::Persist(PersistError::Truncated { line: 2 })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
