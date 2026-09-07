//! Process-level tool approval (Codex-shaped, thin).
//!
//! `THESEUS_TOOL_APPROVAL=auto|approve` (default `approve`; `PI_TOOL_APPROVAL`
//! is a temporary fallback). In `approve`, `write` / `edit` / `bash` wait for
//! `tool/approve` or `tool/reject`. `read` is always automatic. Path fence
//! still applies after a grant. Unanswered requests expire as reject
//! (`THESEUS_TOOL_APPROVAL_TIMEOUT_SECS`, default 60; `PI_*` fallback).

use serde_json::{json, Value};
use std::time::Duration;

pub const ENV_APPROVAL: &str = "THESEUS_TOOL_APPROVAL";
pub const ENV_APPROVAL_LEGACY: &str = "PI_TOOL_APPROVAL";
pub const ENV_APPROVAL_TIMEOUT: &str = "THESEUS_TOOL_APPROVAL_TIMEOUT_SECS";
pub const ENV_APPROVAL_TIMEOUT_LEGACY: &str = "PI_TOOL_APPROVAL_TIMEOUT_SECS";
pub const DEFAULT_APPROVAL_TIMEOUT_SECS: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalMode {
    Auto,
    Approve,
}

impl ApprovalMode {
    pub fn from_env() -> Self {
        match theseus_core::first_nonempty_env(&[ENV_APPROVAL, ENV_APPROVAL_LEGACY])
            .as_deref()
            .map(str::trim)
        {
            Some("auto") => Self::Auto,
            _ => Self::Approve,
        }
    }

    /// Extra gate on top of the path fence. `read` never waits.
    pub fn requires_approval(self, tool: &str) -> bool {
        match self {
            Self::Auto => false,
            Self::Approve => matches!(tool, "write" | "edit" | "bash"),
        }
    }
}

pub fn timeout_from_env() -> Duration {
    let secs =
        theseus_core::first_nonempty_env(&[ENV_APPROVAL_TIMEOUT, ENV_APPROVAL_TIMEOUT_LEGACY])
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_APPROVAL_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

pub fn parse_arguments(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!({ "raw": raw }))
}

pub fn approval_summary(name: &str, arguments: &str) -> String {
    let args = parse_arguments(arguments);
    match name {
        "write" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("?");
            let n = args
                .get("content")
                .and_then(Value::as_str)
                .map(|s| s.len())
                .unwrap_or(0);
            format!("write {path} ({n} bytes)")
        }
        "edit" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("?");
            format!("edit {path}")
        }
        "bash" => {
            let cmd = args.get("command").and_then(Value::as_str).unwrap_or("?");
            let short: String = cmd.chars().take(120).collect();
            if cmd.chars().count() > 120 {
                format!("bash {short}…")
            } else {
                format!("bash {short}")
            }
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approve_gates_dangerous_only() {
        assert!(!ApprovalMode::Approve.requires_approval("read"));
        assert!(ApprovalMode::Approve.requires_approval("write"));
        assert!(ApprovalMode::Approve.requires_approval("edit"));
        assert!(ApprovalMode::Approve.requires_approval("bash"));
        assert!(!ApprovalMode::Auto.requires_approval("write"));
    }

    #[test]
    fn summary_mentions_path_or_command() {
        assert_eq!(
            approval_summary("write", r#"{"path":"out.txt","content":"hi"}"#),
            "write out.txt (2 bytes)"
        );
        assert_eq!(approval_summary("edit", r#"{"path":"a.rs"}"#), "edit a.rs");
        assert_eq!(
            approval_summary("bash", r#"{"command":"echo hi"}"#),
            "bash echo hi"
        );
    }
}
