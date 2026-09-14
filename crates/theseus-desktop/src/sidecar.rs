use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::jsonl::read_jsonl;
use crate::locate::{locate_pi, LocateError};

const GET_STATE_LINE: &str = r#"{"id":0,"type":"get_state"}"#;

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error(transparent)]
    Locate(#[from] LocateError),
    #[error("spawn {path}: {source}")]
    Spawn {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("sidecar stdin closed")]
    StdinClosed,
    #[error("pi --mode rpc did not answer get_state")]
    InitializeTimeout,
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

struct Inner {
    child: Mutex<Child>,
    stdin: Mutex<Option<std::process::ChildStdin>>,
    pid: u32,
    path: PathBuf,
}

/// One `pi --mode rpc` child. JSONL commands on stdin, events/responses on stdout.
#[derive(Clone)]
pub struct Sidecar {
    inner: Arc<Inner>,
}

impl Sidecar {
    pub fn start() -> Result<(Self, Receiver<String>), SidecarError> {
        let path = locate_pi()?;
        Self::spawn(&path)
    }

    pub fn spawn(path: &Path) -> Result<(Self, Receiver<String>), SidecarError> {
        let mut cmd = Command::new(path);
        cmd.arg("--mode").arg("rpc");
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        if crate::verbose_stdio() {
            cmd.stderr(Stdio::inherit());
        } else {
            cmd.stderr(Stdio::null());
        }
        let workspace = crate::resolve_user_workspace();
        if !workspace.is_empty() {
            cmd.current_dir(&workspace);
        }
        debug_assert!(
            cmd.get_args().all(|a| {
                let s = a.to_string_lossy();
                !s.contains("THESEUS_LLM") && !s.contains("sk-") && !s.contains("API_KEY")
            }),
            "sidecar argv must not carry secrets"
        );
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            // A GUI-subsystem parent spawning a console child would otherwise
            // flash a black console. Sidecar I/O is piped; it does not need one.
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|source| SidecarError::Spawn {
            path: path.display().to_string(),
            source,
        })?;
        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SidecarError::Io(std::io::Error::other("sidecar stdin missing")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| SidecarError::Io(std::io::Error::other("sidecar stdout missing")))?;

        let (tx, rx) = mpsc::channel::<String>();
        thread::Builder::new()
            .name("theseus-sidecar-stdout".into())
            .spawn(move || read_jsonl(stdout, tx))
            .map_err(SidecarError::Io)?;

        let sidecar = Self {
            inner: Arc::new(Inner {
                child: Mutex::new(child),
                stdin: Mutex::new(Some(stdin)),
                pid,
                path: path.to_path_buf(),
            }),
        };
        sidecar.handshake(&rx)?;
        Ok((sidecar, rx))
    }

    fn handshake(&self, rx: &Receiver<String>) -> Result<(), SidecarError> {
        self.send_line(GET_STATE_LINE)?;
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline {
            let remain = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remain.min(Duration::from_millis(200))) {
                Ok(line) => {
                    if is_get_state_response(&line) {
                        return Ok(());
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(SidecarError::InitializeTimeout);
                }
            }
        }
        Err(SidecarError::InitializeTimeout)
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    pub fn pid(&self) -> u32 {
        self.inner.pid
    }

    pub fn send_line(&self, line: &str) -> Result<(), SidecarError> {
        let mut guard = self
            .inner
            .stdin
            .lock()
            .map_err(|_| SidecarError::StdinClosed)?;
        let stdin = guard.as_mut().ok_or(SidecarError::StdinClosed)?;
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        Ok(())
    }

    /// Close stdin and kill the process group if it lingers. No Theseus `shutdown` RPC.
    pub fn shutdown(&self) {
        {
            let mut guard = match self.inner.stdin.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            *guard = None;
        }

        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            if !self.still_alive() {
                return;
            }
            thread::sleep(Duration::from_millis(40));
        }
        self.kill_group();
        let _ = self.inner.child.lock().ok().and_then(|mut c| c.wait().ok());
    }

    fn still_alive(&self) -> bool {
        match self.inner.child.lock() {
            Ok(mut child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) => false,
                Err(_) => true,
            },
            Err(_) => true,
        }
    }

    fn kill_group(&self) {
        if !self.still_alive() {
            return;
        }
        let pid = self.inner.pid;
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-TERM", &format!("-{pid}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            thread::sleep(Duration::from_millis(150));
            if self.still_alive() {
                let _ = Command::new("kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .status();
        }
        if let Ok(mut child) = self.inner.child.lock() {
            let _ = child.kill();
        }
    }
}

fn is_get_state_response(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    if v.get("id") != Some(&serde_json::json!(0)) {
        return false;
    }
    v.get("type").and_then(|t| t.as_str()) == Some("response")
        || v.get("command").and_then(|t| t.as_str()) == Some("get_state")
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.shutdown();
        }
    }
}
