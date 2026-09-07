//! Append-only session log (memory + optional JSONL on disk).
//!
//! Invariants:
//! - Model-visible ⇔ already appended (`derive_messages` reads the log only).
//! - Model-visible tool calls come only from `tool/call` events.
//! - Codex Items are projected from the same events (`project_item`).
//! - `fork` copies a prefix of the log.
//! - Turn/step start–end fencing is enforced at append time.
//! - Same turn is hard-capped at [`DEFAULT_MAX_STEPS_PER_TURN`] (20) `step/start`s.
//! - On-disk JSONL is the same event log; restart replays it. No sidecar messages.

mod derive;
mod fence;
mod project;
mod session;
mod store;

pub use derive::derive_messages;
pub use fence::{FenceError, DEFAULT_MAX_STEPS_PER_TURN};
pub use project::{project_item, project_items, project_items_for_turn};
pub use session::{preview_from_events, Session, SessionError};
pub use store::{
    default_sessions_dir, first_nonempty_env, list_session_files, read_jsonl, resolve_sessions_dir,
    session_log_path, PersistError, SessionFile, SessionLog, ENV_HOME, ENV_HOME_LEGACY,
    ENV_SESSIONS_DIR, ENV_SESSIONS_DIR_LEGACY,
};
