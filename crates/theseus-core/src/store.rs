//! Append-only JSONL session log on disk.
//!
//! The file is the same event log as memory — never a derived message list.
//! One thread_id maps to one `{sessions_dir}/{thread_id}.jsonl`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use theseus_protocol::SessionEvent;
use thiserror::Error;

/// Override the sessions directory entirely (`{THESEUS_SESSIONS_DIR}/{thread_id}.jsonl`).
pub const ENV_SESSIONS_DIR: &str = "THESEUS_SESSIONS_DIR";
/// Temporary read fallback for [`ENV_SESSIONS_DIR`].
pub const ENV_SESSIONS_DIR_LEGACY: &str = "PI_SESSIONS_DIR";
/// If `THESEUS_SESSIONS_DIR` is unset, sessions live at `{THESEUS_HOME}/sessions/`.
pub const ENV_HOME: &str = "THESEUS_HOME";
/// Temporary read fallback for [`ENV_HOME`].
pub const ENV_HOME_LEGACY: &str = "PI_HOME";

const DEFAULT_DIR_NAME: &str = ".theseus";
const LEGACY_DIR_NAME: &str = ".pi-app";
const SESSIONS_SUBDIR: &str = "sessions";

/// First non-empty environment value among `names` (canonical name first).
pub fn first_nonempty_env(names: &[&str]) -> Option<String> {
    for name in names {
        if let Ok(v) = std::env::var(name) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PersistError {
    #[error("invalid thread id {0:?}: must be [A-Za-z0-9_-] and not a path")]
    InvalidThreadId(String),
    #[error("session log io error: {0}")]
    Io(String),
    #[error("session log line {line} is not valid JSON: {message}")]
    InvalidLine { line: usize, message: String },
    #[error("session log line {line} is truncated or incomplete")]
    Truncated { line: usize },
    #[error("session log line {line} is not a history event ({found})")]
    NotHistory { line: usize, found: String },
    #[error("session log is empty")]
    Empty,
    #[error("session file id {file:?} does not match thread/meta {meta:?}")]
    ThreadIdMismatch { file: String, meta: String },
    #[error("cannot resolve home directory for default sessions path")]
    NoHome,
}

impl From<io::Error> for PersistError {
    fn from(err: io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

/// Resolve the sessions directory.
///
/// Order: `THESEUS_SESSIONS_DIR` → `PI_SESSIONS_DIR` → `{THESEUS_HOME}/sessions`
/// → `{PI_HOME}/sessions` → `~/.theseus/sessions` (if present) →
/// `~/.pi-app/sessions` (if present) → new `~/.theseus/sessions`.
pub fn resolve_sessions_dir() -> Result<PathBuf, PersistError> {
    if let Some(dir) = first_nonempty_env(&[ENV_SESSIONS_DIR, ENV_SESSIONS_DIR_LEGACY]) {
        return Ok(PathBuf::from(dir));
    }
    if let Some(home) = first_nonempty_env(&[ENV_HOME, ENV_HOME_LEGACY]) {
        return Ok(PathBuf::from(home).join(SESSIONS_SUBDIR));
    }
    let home = std::env::var("HOME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or(PersistError::NoHome)?;
    let home = PathBuf::from(home);
    let canonical = home.join(DEFAULT_DIR_NAME).join(SESSIONS_SUBDIR);
    let legacy = home.join(LEGACY_DIR_NAME).join(SESSIONS_SUBDIR);
    if canonical.is_dir() {
        return Ok(canonical);
    }
    if legacy.is_dir() {
        return Ok(legacy);
    }
    Ok(canonical)
}

/// Like [`resolve_sessions_dir`], but last-resort `/tmp/theseus/sessions` if HOME is missing.
pub fn default_sessions_dir() -> PathBuf {
    resolve_sessions_dir().unwrap_or_else(|_| PathBuf::from("/tmp/theseus/sessions"))
}

pub fn validate_thread_id(thread_id: &str) -> Result<(), PersistError> {
    if thread_id.is_empty()
        || thread_id.len() > 128
        || thread_id.contains('/')
        || thread_id.contains('\\')
        || thread_id.contains("..")
        || !thread_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(PersistError::InvalidThreadId(thread_id.to_string()));
    }
    Ok(())
}

/// One `*.jsonl` in the sessions directory (mtime only; contents unread).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFile {
    pub thread_id: String,
    pub path: PathBuf,
    /// Unix epoch seconds from the file's mtime.
    pub updated_at: i64,
}

/// Scan `{sessions_dir}/*.jsonl` by mtime (newest first). No index.
pub fn list_session_files(sessions_dir: impl AsRef<Path>) -> Vec<SessionFile> {
    let mut out = Vec::new();
    let rd = match fs::read_dir(sessions_dir.as_ref()) {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(stem) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if validate_thread_id(&stem).is_err() {
            continue;
        }
        let updated_at = ent
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        let path = fs::canonicalize(&path).unwrap_or(path);
        out.push(SessionFile {
            thread_id: stem,
            path,
            updated_at,
        });
    }
    out.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.thread_id.cmp(&a.thread_id))
    });
    out
}

/// `{sessions_dir}/{thread_id}.jsonl`
pub fn session_log_path(
    sessions_dir: impl AsRef<Path>,
    thread_id: &str,
) -> Result<PathBuf, PersistError> {
    validate_thread_id(thread_id)?;
    Ok(sessions_dir.as_ref().join(format!("{thread_id}.jsonl")))
}

/// On-disk JSONL handle for one thread. Opens the file per append (create + append).
#[derive(Debug, Clone)]
pub struct SessionLog {
    path: PathBuf,
}

impl SessionLog {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Claim `{dir}/{thread_id}.jsonl` (fails if the file already exists).
    pub fn create(sessions_dir: impl AsRef<Path>, thread_id: &str) -> Result<Self, PersistError> {
        let path = session_log_path(sessions_dir, thread_id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self { path })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<(Self, Vec<SessionEvent>), PersistError> {
        let path = path.as_ref().to_path_buf();
        let events = read_jsonl(&path)?;
        Ok((Self { path }, events))
    }

    pub fn open_thread(
        sessions_dir: impl AsRef<Path>,
        thread_id: &str,
    ) -> Result<(Self, Vec<SessionEvent>), PersistError> {
        let path = session_log_path(sessions_dir, thread_id)?;
        Self::open(path)
    }

    /// Append one history event as a single JSON line and fsync. Never writes chunks.
    pub fn append(&self, event: &SessionEvent) -> Result<(), PersistError> {
        if !event.is_history() {
            return Err(PersistError::NotHistory {
                line: 0,
                found: event.type_name().to_string(),
            });
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut file, event).map_err(|err| PersistError::Io(err.to_string()))?;
        file.write_all(b"\n")?;
        file.flush()?;
        file.sync_all()?;
        Ok(())
    }
}

pub fn read_jsonl(path: impl AsRef<Path>) -> Result<Vec<SessionEvent>, PersistError> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)?;
    if text.is_empty() {
        return Err(PersistError::Empty);
    }

    let mut events = Vec::new();
    let parts: Vec<&str> = text.split('\n').collect();
    for (i, line) in parts.iter().enumerate() {
        let line_no = i + 1;
        if line.is_empty() {
            if i + 1 == parts.len() {
                continue;
            }
            return Err(PersistError::InvalidLine {
                line: line_no,
                message: "empty line".into(),
            });
        }
        let event: SessionEvent = match serde_json::from_str(line) {
            Ok(ev) => ev,
            Err(err) => {
                if looks_truncated(line, &err) {
                    return Err(PersistError::Truncated { line: line_no });
                }
                return Err(PersistError::InvalidLine {
                    line: line_no,
                    message: err.to_string(),
                });
            }
        };
        if !event.is_history() {
            return Err(PersistError::NotHistory {
                line: line_no,
                found: event.type_name().to_string(),
            });
        }
        events.push(event);
    }
    if events.is_empty() {
        return Err(PersistError::Empty);
    }
    Ok(events)
}

fn looks_truncated(line: &str, err: &serde_json::Error) -> bool {
    let msg = err.to_string();
    msg.contains("EOF")
        || msg.contains("eof")
        || msg.contains("unexpected end")
        || !brace_balanced(line)
}

fn brace_balanced(line: &str) -> bool {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for c in line.chars() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '\\' if in_str => escape = true,
            '"' => in_str = !in_str,
            '{' if !in_str => depth += 1,
            '}' if !in_str => depth -= 1,
            _ => {}
        }
    }
    depth == 0 && !in_str
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use theseus_protocol::{
        AssistantChunk, EventData, ThreadMeta, TurnEnd, TurnEndReason, TurnStart, UserMessageEvent,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "theseus-core-store-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn meta_event(id: &str) -> SessionEvent {
        SessionEvent {
            seq: 0,
            time: 1,
            data: EventData::ThreadMeta(ThreadMeta {
                thread_id: id.into(),
                model: Some("stub".into()),
                cwd: None,
                title: None,
                created_at: 1,
                parent_thread_id: None,
            }),
        }
    }

    #[test]
    fn list_session_files_sorts_by_mtime_and_skips_other_files() {
        let dir = temp_dir("list");
        fs::write(dir.join("notes.txt"), "nope").unwrap();
        let a = SessionLog::create(&dir, "thr_a").unwrap();
        a.append(&meta_event("thr_a")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let b = SessionLog::create(&dir, "thr_b").unwrap();
        b.append(&meta_event("thr_b")).unwrap();
        let listed = list_session_files(&dir);
        assert_eq!(
            listed
                .iter()
                .map(|f| f.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["thr_b", "thr_a"]
        );
        assert!(listed[0].path.ends_with("thr_b.jsonl"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn session_log_path_is_one_file_per_thread() {
        let path = session_log_path("/data/sessions", "thr_1").unwrap();
        assert_eq!(path, PathBuf::from("/data/sessions/thr_1.jsonl"));
        assert!(session_log_path("/data", "../etc").is_err());
        assert!(session_log_path("/data", "thr/1").is_err());
        assert!(session_log_path("/data", "").is_err());
    }

    fn clear_session_env() {
        std::env::remove_var(ENV_SESSIONS_DIR);
        std::env::remove_var(ENV_SESSIONS_DIR_LEGACY);
        std::env::remove_var(ENV_HOME);
        std::env::remove_var(ENV_HOME_LEGACY);
    }

    #[test]
    fn resolve_prefers_sessions_dir_then_home() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_session_env();
        std::env::set_var(ENV_SESSIONS_DIR, "/custom/sess");
        std::env::set_var(ENV_HOME, "/custom/home");
        assert_eq!(
            resolve_sessions_dir().unwrap(),
            PathBuf::from("/custom/sess")
        );
        std::env::remove_var(ENV_SESSIONS_DIR);
        assert_eq!(
            resolve_sessions_dir().unwrap(),
            PathBuf::from("/custom/home/sessions")
        );
        std::env::remove_var(ENV_HOME);
        let got = resolve_sessions_dir().unwrap();
        let home = std::env::var("HOME").unwrap();
        let canonical = PathBuf::from(&home).join(".theseus/sessions");
        let legacy = PathBuf::from(&home).join(".pi-app/sessions");
        if canonical.is_dir() {
            assert_eq!(got, canonical);
        } else if legacy.is_dir() {
            assert_eq!(got, legacy);
        } else {
            assert_eq!(got, canonical);
        }
        clear_session_env();
    }

    #[test]
    fn resolve_falls_back_to_legacy_pi_env() {
        let _g = ENV_LOCK.lock().unwrap();
        clear_session_env();
        std::env::set_var(ENV_SESSIONS_DIR_LEGACY, "/legacy/sess");
        std::env::set_var(ENV_HOME_LEGACY, "/legacy/home");
        assert_eq!(
            resolve_sessions_dir().unwrap(),
            PathBuf::from("/legacy/sess")
        );
        std::env::remove_var(ENV_SESSIONS_DIR_LEGACY);
        assert_eq!(
            resolve_sessions_dir().unwrap(),
            PathBuf::from("/legacy/home/sessions")
        );
        std::env::set_var(ENV_SESSIONS_DIR, "/new/sess");
        assert_eq!(resolve_sessions_dir().unwrap(), PathBuf::from("/new/sess"));
        clear_session_env();
    }

    #[test]
    fn append_and_read_roundtrip() {
        let dir = temp_dir("round");
        let log = SessionLog::create(&dir, "thr_1").unwrap();
        let a = meta_event("thr_1");
        let b = SessionEvent {
            seq: 1,
            time: 2,
            data: EventData::TurnStart(TurnStart { turn: 1 }),
        };
        log.append(&a).unwrap();
        log.append(&b).unwrap();
        let loaded = read_jsonl(log.path()).unwrap();
        assert_eq!(loaded, vec![a, b]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn create_is_exclusive_per_thread_id() {
        let dir = temp_dir("excl");
        SessionLog::create(&dir, "thr_1").unwrap();
        let err = SessionLog::create(&dir, "thr_1").unwrap_err();
        assert!(matches!(err, PersistError::Io(_)), "{err:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn chunk_is_never_written() {
        let dir = temp_dir("chunk");
        let log = SessionLog::create(&dir, "thr_1").unwrap();
        let err = log
            .append(&SessionEvent {
                seq: 0,
                time: 1,
                data: EventData::AssistantChunk(AssistantChunk {
                    turn: 1,
                    step: 1,
                    text: "x".into(),
                }),
            })
            .unwrap_err();
        assert!(
            matches!(err, PersistError::NotHistory { found, .. } if found == "assistant/chunk")
        );
        assert_eq!(fs::read_to_string(log.path()).unwrap(), "");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_line_is_load_error() {
        let dir = temp_dir("bad");
        let path = dir.join("thr_1.jsonl");
        let meta = serde_json::to_string(&meta_event("thr_1")).unwrap();
        fs::write(&path, format!("{meta}\nNOT JSON\n")).unwrap();
        let err = read_jsonl(&path).unwrap_err();
        assert!(
            matches!(err, PersistError::InvalidLine { line: 2, .. }),
            "{err:?}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn truncated_line_is_load_error() {
        let dir = temp_dir("trunc");
        let path = dir.join("thr_1.jsonl");
        let meta = serde_json::to_string(&meta_event("thr_1")).unwrap();
        fs::write(
            &path,
            format!("{meta}\n{{\"type\":\"turn/start\",\"seq\":1"),
        )
        .unwrap();
        let err = read_jsonl(&path).unwrap_err();
        assert!(
            matches!(err, PersistError::Truncated { line: 2 }),
            "{err:?}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn chunk_in_file_is_load_error() {
        let dir = temp_dir("chunk-load");
        let path = dir.join("thr_1.jsonl");
        let meta = serde_json::to_string(&meta_event("thr_1")).unwrap();
        let chunk = serde_json::to_string(&SessionEvent {
            seq: 1,
            time: 2,
            data: EventData::AssistantChunk(AssistantChunk {
                turn: 1,
                step: 1,
                text: "x".into(),
            }),
        })
        .unwrap();
        fs::write(&path, format!("{meta}\n{chunk}\n")).unwrap();
        let err = read_jsonl(&path).unwrap_err();
        assert!(matches!(
            err,
            PersistError::NotHistory {
                line: 2,
                found
            } if found == "assistant/chunk"
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn persist_io_failure_is_explicit() {
        let dir = temp_dir("iofail");
        let log = SessionLog::create(&dir, "thr_1").unwrap();
        fs::remove_file(log.path()).unwrap();
        fs::create_dir(log.path()).unwrap();
        let err = log.append(&meta_event("thr_1")).unwrap_err();
        assert!(matches!(err, PersistError::Io(_)), "{err:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn file_contains_only_event_json_not_derived_messages() {
        let dir = temp_dir("no-msg");
        let log = SessionLog::create(&dir, "thr_1").unwrap();
        log.append(&meta_event("thr_1")).unwrap();
        log.append(&SessionEvent {
            seq: 1,
            time: 2,
            data: EventData::TurnStart(TurnStart { turn: 1 }),
        })
        .unwrap();
        log.append(&SessionEvent {
            seq: 2,
            time: 3,
            data: EventData::UserMessage(UserMessageEvent {
                turn: 1,
                id: "item_u".into(),
                content: "hello".into(),
                source: Some("user".into()),
            }),
        })
        .unwrap();
        log.append(&SessionEvent {
            seq: 3,
            time: 4,
            data: EventData::TurnEnd(TurnEnd {
                turn: 1,
                reason: TurnEndReason::Completed,
            }),
        })
        .unwrap();
        let text = fs::read_to_string(log.path()).unwrap();
        assert!(!text.contains("\"role\""));
        assert!(!text.contains("derived"));
        assert_eq!(text.lines().count(), 4);
        let _ = fs::remove_dir_all(dir);
    }
}
