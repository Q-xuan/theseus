//! Frozen SessionEvent vocabulary plus the outward Thread / Turn / Item shapes.
//!
//! History is the nine kinds listed in [`HISTORY_EVENT_TYPES`]. `assistant/chunk`
//! exists only for streaming UX and is **not** a history event.

mod event;
mod message;
mod schema;
mod wire;

pub use event::*;
pub use message::*;
pub use schema::{session_event_json_schema, session_event_schema};
pub use wire::*;

/// The nine SessionEvent kinds that may appear in the append-only history log.
pub const HISTORY_EVENT_TYPES: &[&str] = &[
    "thread/meta",
    "turn/start",
    "turn/end",
    "step/start",
    "step/end",
    "user/message",
    "assistant/message",
    "tool/call",
    "tool/result",
];

/// Streaming-only type. Must not enter history / `derive_messages`.
pub const STREAMING_EVENT_TYPE: &str = "assistant/chunk";
