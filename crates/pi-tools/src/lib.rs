//! In-process tools for the same-turn loop.
//!
//! Cwd is supplied by the caller (the thread workspace). File paths are
//! resolved (including `..` and symlinks) and must stay under that root.
//! There is no sandbox: `bash` still always has a timeout.

mod bash;
mod fs;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use serde_json::json;

pub use bash::{DEFAULT_TIMEOUT_SECS, MAX_TIMEOUT_SECS};

/// One tool invocation against a pinned workspace.
pub trait ToolSeam: Send {
    fn execute(&self, name: &str, arguments: &str, ctx: &ToolContext) -> ToolOutput;
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Process-local implementations of `read` / `write` / `edit` / `bash`.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalToolSeam;

impl ToolSeam for LocalToolSeam {
    fn execute(&self, name: &str, arguments: &str, ctx: &ToolContext) -> ToolOutput {
        match name {
            "read" => fs::read_file(arguments, ctx),
            "write" => fs::write_file(arguments, ctx),
            "edit" => fs::edit_file(arguments, ctx),
            "bash" => bash::run(arguments, ctx),
            other => ToolOutput::error(format!("unknown tool: {other}")),
        }
    }
}

/// OpenAI-style function schemas for the four tools.
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read".into(),
            description: "Read a text file inside the thread workspace. After resolving `..` and symlinks the path must stay under the workspace root. `offset` is a 1-based line number; `limit` is how many lines to return.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "offset": { "type": "integer", "description": "1-based start line" },
                    "limit": { "type": "integer", "description": "Maximum number of lines" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "write".into(),
            description: "Create or overwrite a text file inside the thread workspace. Parent directories are created as needed. Paths that resolve outside the workspace are rejected.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" },
                    "content": { "type": "string", "description": "Full file contents" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "edit".into(),
            description: "Replace `oldString` with `newString` in a file inside the thread workspace. Fails if `oldString` is missing. Without `replaceAll`, fails if it matches more than once. Aliases: oldText/newText.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "oldString": { "type": "string" },
                    "newString": { "type": "string" },
                    "replaceAll": { "type": "boolean", "description": "Replace every match (default false)" }
                },
                "required": ["path", "oldString", "newString"]
            }),
        },
        ToolDefinition {
            name: "bash".into(),
            description: format!(
                "Run a shell command in the thread workspace. `timeout` is seconds (default {DEFAULT_TIMEOUT_SECS}, max {MAX_TIMEOUT_SECS}). No sandbox; the timeout still applies."
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "timeout": { "type": "number", "description": "Deadline in seconds" }
                },
                "required": ["command"]
            }),
        },
    ]
}

/// Canonical workspace root. Used as bash `current_dir` and as the prefix
/// that every file-tool path must stay under.
pub(crate) fn pin_workspace(cwd: &Path) -> Result<PathBuf, String> {
    let root = cwd
        .canonicalize()
        .map_err(|e| format!("workspace root is not accessible: {}: {e}", cwd.display()))?;
    if !root.is_dir() {
        return Err(format!(
            "workspace root is not a directory: {}",
            root.display()
        ));
    }
    Ok(root)
}

/// Resolve `path` against the workspace and require the result (after `..`
/// and symlink follow) to stay under that root.
pub(crate) fn resolve_path(cwd: &Path, path: &str) -> Result<PathBuf, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("path is empty".into());
    }
    let root = pin_workspace(cwd)?;
    let requested = Path::new(path);
    let joined = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let resolved = canonicalize_existing_prefix(&joined)?;
    if !resolved.starts_with(&root) {
        return Err(format!("path escapes workspace: {path}"));
    }
    Ok(resolved)
}

/// `realpath` the longest existing prefix (follows symlinks), then apply the
/// remaining components lexically so new files can still be created inside.
fn canonicalize_existing_prefix(path: &Path) -> Result<PathBuf, String> {
    if let Ok(canon) = path.canonicalize() {
        return Ok(canon);
    }
    let mut rest: Vec<OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        if let Ok(canon) = cursor.canonicalize() {
            let mut out = canon;
            for part in rest.iter().rev() {
                if part == OsStr::new("..") {
                    let _ = out.pop();
                } else if part != OsStr::new(".") && !part.is_empty() {
                    out.push(part);
                }
            }
            return Ok(out);
        }
        match cursor.file_name() {
            Some(name) => {
                rest.push(name.to_os_string());
                if !cursor.pop() {
                    return Err(format!("cannot resolve path: {}", path.display()));
                }
            }
            None => return Err(format!("cannot resolve path: {}", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Instant;

    fn ctx_dir(name: &str) -> (ToolContext, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "pi-tools-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        (ToolContext { cwd: dir.clone() }, dir)
    }

    #[test]
    fn read_write_edit_round_trip() {
        let seam = LocalToolSeam;
        let (ctx, dir) = ctx_dir("fs");
        let wrote = seam.execute(
            "write",
            r#"{"path":"note.txt","content":"alpha\nalpha\nbeta\n"}"#,
            &ctx,
        );
        assert!(!wrote.is_error, "{}", wrote.content);
        assert!(dir.join("note.txt").is_file());

        let read = seam.execute("read", r#"{"path":"note.txt","offset":2,"limit":1}"#, &ctx);
        assert!(!read.is_error, "{}", read.content);
        assert!(read.content.contains("alpha"));
        assert!(read.content.contains("2|"));

        let bad = seam.execute(
            "edit",
            r#"{"path":"note.txt","oldString":"alpha","newString":"ALPHA"}"#,
            &ctx,
        );
        assert!(bad.is_error, "ambiguous edit should fail");

        let edited = seam.execute(
            "edit",
            r#"{"path":"note.txt","oldString":"alpha","newString":"ALPHA","replaceAll":true}"#,
            &ctx,
        );
        assert!(!edited.is_error, "{}", edited.content);
        let body = fs::read_to_string(dir.join("note.txt")).unwrap();
        assert_eq!(body, "ALPHA\nALPHA\nbeta\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn bash_runs_in_workspace() {
        let seam = LocalToolSeam;
        let (ctx, dir) = ctx_dir("bash");
        let out = seam.execute("bash", r#"{"command":"pwd"}"#, &ctx);
        assert!(!out.is_error, "{}", out.content);
        let got = out.content.trim();
        let expected = dir.canonicalize().unwrap_or(dir.clone());
        assert!(
            got.contains(&expected.to_string_lossy().to_string())
                || got.contains(&dir.to_string_lossy().to_string()),
            "pwd={got} dir={}",
            dir.display()
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn bash_deadline_stops_child() {
        let seam = LocalToolSeam;
        let (ctx, dir) = ctx_dir("deadline");
        let start = Instant::now();
        let out = seam.execute(
            "bash",
            r#"{"command":"while true; do true; done","timeout":0.25}"#,
            &ctx,
        );
        let elapsed = start.elapsed();
        assert!(out.is_error, "deadline must be an error");
        assert!(out.content.contains("timed out"), "content={}", out.content);
        assert!(
            elapsed.as_secs_f64() < 2.0,
            "deadline wait took {:?}",
            elapsed
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unknown_tool_is_error() {
        let (ctx, dir) = ctx_dir("unk");
        let out = LocalToolSeam.execute("nope", "{}", &ctx);
        assert!(out.is_error);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn relative_dotdot_escape_is_error() {
        let seam = LocalToolSeam;
        let (ctx, dir) = ctx_dir("esc-rel");
        let outside = dir.parent().expect("temp parent").join(format!(
            "pi-outside-rel-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        fs::write(&outside, "secret-rel").unwrap();
        let name = outside.file_name().unwrap().to_string_lossy();
        let args = format!(r#"{{"path":"../{name}"}}"#);

        for tool in ["read", "edit"] {
            let payload = if tool == "edit" {
                format!(r#"{{"path":"../{name}","oldString":"secret-rel","newString":"x"}}"#)
            } else {
                args.clone()
            };
            let out = seam.execute(tool, &payload, &ctx);
            assert!(
                out.is_error,
                "{tool} should reject ../ escape: {}",
                out.content
            );
            assert!(
                out.content.contains("escapes workspace"),
                "{tool} content={}",
                out.content
            );
            assert!(
                !out.content.contains("secret-rel"),
                "{tool} must not leak outside file"
            );
        }
        let wrote = seam.execute(
            "write",
            &format!(r#"{{"path":"../{name}.new","content":"nope"}}"#),
            &ctx,
        );
        assert!(wrote.is_error, "{}", wrote.content);
        assert!(wrote.content.contains("escapes workspace"));
        assert!(!dir.parent().unwrap().join(format!("{name}.new")).exists());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "secret-rel");
        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn absolute_outside_is_error() {
        let seam = LocalToolSeam;
        let (ctx, dir) = ctx_dir("esc-abs");
        let out = seam.execute("read", r#"{"path":"/etc/passwd"}"#, &ctx);
        assert!(out.is_error, "{}", out.content);
        assert!(out.content.contains("escapes workspace"), "{}", out.content);
        assert!(
            !out.content.contains("root:"),
            "must not return /etc/passwd contents: {}",
            out.content
        );
        let wrote = seam.execute("write", r#"{"path":"/etc/passwd","content":"nope"}"#, &ctx);
        assert!(wrote.is_error, "{}", wrote.content);
        assert!(wrote.content.contains("escapes workspace"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn in_workspace_relative_ok() {
        let seam = LocalToolSeam;
        let (ctx, dir) = ctx_dir("ok-rel");
        fs::create_dir_all(dir.join("sub")).unwrap();
        let wrote = seam.execute(
            "write",
            r#"{"path":"sub/note.txt","content":"inside"}"#,
            &ctx,
        );
        assert!(!wrote.is_error, "{}", wrote.content);
        let read = seam.execute("read", r#"{"path":"sub/note.txt"}"#, &ctx);
        assert!(!read.is_error, "{}", read.content);
        assert!(read.content.contains("inside"));
        let abs = dir.join("sub/note.txt").canonicalize().unwrap();
        let via_abs = seam.execute("read", &format!(r#"{{"path":"{}"}}"#, abs.display()), &ctx);
        assert!(
            !via_abs.is_error,
            "in-workspace absolute must be ok: {}",
            via_abs.content
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn symlink_outside_is_error() {
        let seam = LocalToolSeam;
        let (ctx, dir) = ctx_dir("esc-link");
        std::os::unix::fs::symlink("/etc", dir.join("outlink")).unwrap();
        let out = seam.execute("read", r#"{"path":"outlink/passwd"}"#, &ctx);
        assert!(out.is_error, "{}", out.content);
        assert!(out.content.contains("escapes workspace"), "{}", out.content);
        assert!(!out.content.contains("root:"), "{}", out.content);
        let _ = fs::remove_dir_all(dir);
    }

    fn now_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
