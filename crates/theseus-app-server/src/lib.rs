//! stdio JSON-RPC app-server (Codex-shaped Thread / Turn / Item on the wire).
//!
//! `turn/start` runs a same-turn tool loop: `derive_messages` → `LlmSeam` →
//! append `tool/call` (the only model-visible call source) → optional
//! approval gate (`write` / `edit` / `bash`) → `ToolSeam` → `tool/result` →
//! derive again, until a text-only reply or the per-turn step cap. Streaming
//! tokens leave as `item/agentMessage/delta` only; `assistant/chunk` is never
//! appended.
//!
//! Each thread's session log is also appended to
//! `{THESEUS_SESSIONS_DIR|THESEUS_HOME/sessions|~/.theseus/sessions}/{thread_id}.jsonl`
//! (temporary `PI_*` / `~/.pi-app` read fallbacks).
//! That file is the same event log — not a derived message table.
//! `thread/resume` loads the JSONL; `thread/list` scans mtime. No index.

mod approval;
mod rpc;
mod server;

pub use approval::{
    ApprovalMode, DEFAULT_APPROVAL_TIMEOUT_SECS, ENV_APPROVAL, ENV_APPROVAL_TIMEOUT,
};
pub use rpc::{JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse};
pub use server::{AppServer, HandleResult, Outgoing};
