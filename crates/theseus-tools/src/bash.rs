use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::{pin_workspace, ToolContext, ToolOutput};

pub const DEFAULT_TIMEOUT_SECS: f64 = 30.0;
pub const MAX_TIMEOUT_SECS: f64 = 300.0;
const OUTPUT_CAP: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
struct BashArgs {
    command: String,
    #[serde(default)]
    timeout: Option<f64>,
}

pub(crate) fn run(arguments: &str, ctx: &ToolContext) -> ToolOutput {
    let args: BashArgs = match serde_json::from_str(arguments) {
        Ok(a) => a,
        Err(e) => return ToolOutput::error(format!("invalid bash arguments: {e}")),
    };
    if args.command.trim().is_empty() {
        return ToolOutput::error("command is empty");
    }
    let cwd = match pin_workspace(&ctx.cwd) {
        Ok(p) => p,
        Err(e) => return ToolOutput::error(e),
    };
    run_command(&args.command, &cwd, normalize_timeout(args.timeout))
}

fn normalize_timeout(timeout: Option<f64>) -> Duration {
    let secs = timeout.filter(|s| *s > 0.0).unwrap_or(DEFAULT_TIMEOUT_SECS);
    Duration::from_secs_f64(secs.min(MAX_TIMEOUT_SECS))
}

fn run_command(command: &str, cwd: &Path, timeout: Duration) -> ToolOutput {
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolOutput::error(format!("failed to spawn: {e}")),
    };

    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out_h = thread::spawn(move || drain(&mut stdout));
    let err_h = thread::spawn(move || drain(&mut stderr));

    let deadline = Instant::now() + timeout;
    let timed_out = loop {
        match child.try_wait() {
            Ok(Some(_)) => break false,
            Ok(None) => {
                if Instant::now() >= deadline {
                    kill_child(&mut child);
                    break true;
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(e) => return ToolOutput::error(format!("wait failed: {e}")),
        }
    };

    if !timed_out {
        let _ = child.wait();
    }

    let stdout = out_h.join().unwrap_or_default();
    let stderr = err_h.join().unwrap_or_default();
    let mut content = String::new();
    if !stdout.is_empty() {
        content.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        if !stdout.is_empty() {
            content.push_str("stderr:\n");
        }
        content.push_str(&stderr);
    }

    if timed_out {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&format!(
            "bash timed out after {:.3}s",
            timeout.as_secs_f64()
        ));
        return ToolOutput::error(content);
    }

    ToolOutput::ok(if content.is_empty() {
        "(no output)".into()
    } else {
        content
    })
}

fn drain(reader: &mut impl Read) -> String {
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);
    let truncated = buf.len() > OUTPUT_CAP;
    if truncated {
        buf.truncate(OUTPUT_CAP);
    }
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if truncated {
        text.push_str("\n…(truncated)");
    }
    text
}

fn kill_child(child: &mut std::process::Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}
