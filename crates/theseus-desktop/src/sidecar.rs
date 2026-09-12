use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::locate::{locate_app_server, LocateError};

const INIT_LINE: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"clientInfo":{"name":"theseus-desktop","version":"0.7.2"}}}"#;
const SHUTDOWN_LINE: &str = r#"{"jsonrpc":"2.0","id":999999,"method":"shutdown","params":{}}"#;

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
    #[error("sidecar did not answer initialize")]
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

/// One `theseus-app-server` child. JSON-RPC lines on stdin/stdout (Codex-shaped).
#[derive(Clone)]
pub struct Sidecar {
    inner: Arc<Inner>,
}

impl Sidecar {
    pub fn start() -> Result<(Self, Receiver<String>), SidecarError> {
        let path = locate_app_server()?;
        Self::spawn(&path)
    }

    pub fn spawn(path: &Path) -> Result<(Self, Receiver<String>), SidecarError> {
        let mut cmd = Command::new(path);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
        // Product GUI: swallow sidecar stderr (`sessions: …` / locate noise).
        // Preview and debug still inherit so the terminal can show it.
        if crate::verbose_stdio() {
            cmd.stderr(Stdio::inherit());
        } else {
            cmd.stderr(Stdio::null());
        }
        // Inherit the parent environment (including THESEUS_LLM_API_KEY / PI_LLM_API_KEY).
        // Never pass the key as an argument. Model and base URL are non-secret:
        // pin them from env / ~/.theseus so the child matches the settings card.
        crate::hydrate_process_key();
        cmd.env(crate::ENV_MODEL, crate::sidecar_model());
        cmd.env(crate::ENV_BASE_URL, crate::sidecar_base_url());
        debug_assert!(
            cmd.get_args().all(|a| {
                let s = a.to_string_lossy();
                !s.contains("THESEUS_LLM") && !s.contains("PI_LLM") && !s.contains("sk-")
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
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(line) if !line.trim().is_empty() => {
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            })
            .map_err(SidecarError::Io)?;

        let sidecar = Self {
            inner: Arc::new(Inner {
                child: Mutex::new(child),
                stdin: Mutex::new(Some(stdin)),
                pid,
                path: path.to_path_buf(),
            }),
        };
        sidecar.initialize(&rx)?;
        Ok((sidecar, rx))
    }

    fn initialize(&self, rx: &Receiver<String>) -> Result<(), SidecarError> {
        self.send_line(INIT_LINE)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let remain = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remain.min(Duration::from_millis(200))) {
                Ok(line) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                        if v.get("id") == Some(&serde_json::json!(0)) {
                            return Ok(());
                        }
                    }
                    // Unexpected pre-init noise: drop. Do not fan out to UI.
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
        writeln!(stdin, "{line}")?;
        stdin.flush()?;
        Ok(())
    }

    /// Ask the child to exit, then kill the process group if it lingers.
    pub fn shutdown(&self) {
        let _ = self.send_line(SHUTDOWN_LINE);
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

impl Drop for Sidecar {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.shutdown();
        }
    }
}
