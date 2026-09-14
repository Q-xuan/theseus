//! Locate pi session files for the sidebar. Not a second log; just mtime + a light read.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

fn unix_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn file_mtime_ms(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(unix_ms)
        .unwrap_or(0)
}

fn user_home() -> Option<PathBuf> {
    theseus_core::user_home_dir()
}

/// Directories that may hold pi session JSONL files.
pub fn session_dirs(current_file: Option<&str>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(file) = current_file {
        if let Some(parent) = Path::new(file).parent() {
            push_unique(&mut dirs, parent.to_path_buf());
        }
    }
    if let Some(home) = user_home() {
        push_unique(&mut dirs, home.join(".pi").join("agent").join("sessions"));
        push_unique(&mut dirs, home.join(".pi").join("sessions"));
    }
    let cwd = crate::resolve_user_workspace();
    if !cwd.is_empty() {
        push_unique(
            &mut dirs,
            PathBuf::from(&cwd).join(".pi").join("agent").join("sessions"),
        );
        push_unique(&mut dirs, PathBuf::from(&cwd).join(".pi").join("sessions"));
    }
    dirs
}

fn push_unique(dirs: &mut Vec<PathBuf>, path: PathBuf) {
    if !dirs.iter().any(|d| d == &path) {
        dirs.push(path);
    }
}

fn first_user_preview(path: &Path) -> (String, String) {
    let Ok(body) = fs::read_to_string(path) else {
        return (String::new(), String::new());
    };
    let mut title = String::new();
    let mut preview = String::new();
    for (i, line) in body.lines().take(40).enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if title.is_empty() {
            if let Some(name) = v
                .get("name")
                .or_else(|| v.get("sessionName"))
                .and_then(|n| n.as_str())
            {
                if !name.is_empty() {
                    title = name.to_string();
                }
            }
        }
        let message = if v.get("role").and_then(|r| r.as_str()) == Some("user") {
            Some(&v)
        } else {
            v.get("message")
                .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        };
        if let Some(message) = message {
            let text = message_text(message);
            if !text.is_empty() {
                if preview.is_empty() {
                    preview = text;
                }
                if title.is_empty() {
                    title = preview.clone();
                }
                break;
            }
        }
        if i == 0 {
            if let Some(id) = v.get("id").and_then(|n| n.as_str()) {
                if title.is_empty() {
                    title = id.to_string();
                }
            }
        }
    }
    (title, preview)
}

fn message_text(message: &Value) -> String {
    let content = &message["content"];
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

fn stem_id(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session")
        .to_string()
}

/// Sidebar rows: title + updatedAt. Preview is optional.
pub fn list_sessions(current_file: Option<&str>) -> Value {
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for dir in session_dirs(current_file) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(canon) = fs::canonicalize(&path) else {
                continue;
            };
            if !seen.insert(canon.clone()) {
                continue;
            }
            let (title, preview) = first_user_preview(&path);
            let id = stem_id(&path);
            rows.push(json!({
                "id": id,
                "path": path.to_string_lossy(),
                "title": if title.is_empty() { id.clone() } else { title },
                "preview": preview,
                "updatedAt": file_mtime_ms(&path),
            }));
        }
    }
    rows.sort_by(|a, b| {
        let ta = a.get("updatedAt").and_then(|v| v.as_i64()).unwrap_or(0);
        let tb = b.get("updatedAt").and_then(|v| v.as_i64()).unwrap_or(0);
        tb.cmp(&ta)
    });
    rows.truncate(40);
    json!({ "sessions": rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_jsonl_title_and_mtime() {
        let root = std::env::temp_dir().join(format!(
            "theseus-pi-sessions-{}-{}",
            std::process::id(),
            unix_ms(SystemTime::now())
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("abc.jsonl");
        fs::write(
            &path,
            "{\"type\":\"session\",\"name\":\"feature-work\"}\n{\"role\":\"user\",\"content\":\"hello there\"}\n",
        )
        .unwrap();
        let listed = list_sessions(Some(path.to_str().unwrap()));
        let sessions = listed["sessions"].as_array().unwrap();
        assert!(!sessions.is_empty());
        assert_eq!(sessions[0]["id"], "abc");
        assert_eq!(sessions[0]["title"], "feature-work");
        assert_eq!(sessions[0]["preview"], "hello there");
        assert!(sessions[0]["updatedAt"].as_i64().unwrap() > 0);
        let _ = fs::remove_dir_all(&root);
    }
}
