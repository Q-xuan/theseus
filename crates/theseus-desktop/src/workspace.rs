//! Default workspace (cwd) for new `thread/start` calls.
//!
//! Persisted at `~/.theseus/workspace` (a path, not a secret). New threads send
//! this as `thread/start.cwd`. Existing threads keep the cwd stored in
//! `thread/meta`.

use std::fs;
use std::path::{Path, PathBuf};

use theseus_core::user_home_dir;

const CONFIG_DIR_NAME: &str = ".theseus";
const WORKSPACE_FILE_NAME: &str = "workspace";
const MAX_WORKSPACE_CHARS: usize = 4096;


pub fn user_workspace_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(CONFIG_DIR_NAME)
            .join(WORKSPACE_FILE_NAME),
    )
}

fn read_workspace_file() -> Option<String> {
    let path = user_workspace_path()?;
    let raw = fs::read_to_string(path).ok()?;
    normalize_workspace(&raw)
}

/// Trim, reject empty / control / NUL. Relative paths stay relative.
pub fn normalize_workspace(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.chars().count() > MAX_WORKSPACE_CHARS {
        return None;
    }
    if s.chars().any(|c| c.is_control() || c == '\0') {
        return None;
    }
    if s.split(['/', '\\']).any(|part| part == "..") {
        return None;
    }
    Some(s.to_string())
}

fn process_cwd() -> String {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".into())
}

/// File → process cwd.
pub fn resolve_user_workspace() -> String {
    read_workspace_file().unwrap_or_else(process_cwd)
}

/// Persist a workspace directory. Does not have to exist yet (picker / typed).
pub fn persist_user_workspace(raw: &str) -> Result<String, String> {
    let workspace = normalize_workspace(raw).ok_or_else(|| "invalid workspace path".to_string())?;
    let path = user_workspace_path().ok_or_else(|| "cannot resolve home directory".to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&path, format!("{workspace}\n")).map_err(|e| e.to_string())?;
    Ok(workspace)
}

/// Native folder dialog. Returns `None` if the user cancelled or no dialog exists.
pub fn pick_workspace_directory() -> Option<String> {
    pick_directory_native().and_then(|p| normalize_workspace(&p.to_string_lossy()))
}

fn pick_directory_native() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("osascript")
            .args(["-e", "POSIX path of (choose folder)"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            return None;
        }
        return Some(PathBuf::from(s));
    }
    #[cfg(windows)]
    {
        let script = "Add-Type -AssemblyName System.Windows.Forms; \
$d = New-Object System.Windows.Forms.FolderBrowserDialog; \
$d.Description = 'Workspace'; \
if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { $d.SelectedPath }";
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", script])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            return None;
        }
        return Some(PathBuf::from(s));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for (bin, args) in [
            ("zenity", vec!["--file-selection", "--directory"]),
            ("kdialog", vec!["--getexistingdirectory", "."]),
        ] {
            if let Ok(out) = std::process::Command::new(bin).args(&args).output() {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !s.is_empty() {
                        return Some(PathBuf::from(s));
                    }
                }
            }
        }
        None
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[allow(dead_code)]
pub fn workspace_exists(path: &str) -> bool {
    Path::new(path).is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with_temp_home<F: FnOnce(&std::path::Path)>(f: F) {
        let _g = lock_env();
        let home = std::env::temp_dir().join(format!(
            "theseus-workspace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&home).unwrap();
        let old_home = std::env::var("HOME").ok();
        let old_profile = std::env::var("USERPROFILE").ok();
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        f(&home);
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match old_profile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn normalize_rejects_blank_and_escape() {
        assert_eq!(normalize_workspace("  "), None);
        assert_eq!(normalize_workspace("a\nb"), None);
        assert_eq!(normalize_workspace("../secret"), None);
        assert_eq!(
            normalize_workspace("  /tmp/work  "),
            Some("/tmp/work".into())
        );
    }

    #[test]
    fn file_round_trip() {
        with_temp_home(|home| {
            let got = persist_user_workspace("/tmp/theseus-ws").unwrap();
            assert_eq!(got, "/tmp/theseus-ws");
            let path = home.join(".theseus").join("workspace");
            assert_eq!(fs::read_to_string(path).unwrap().trim(), "/tmp/theseus-ws");
            assert_eq!(resolve_user_workspace(), "/tmp/theseus-ws");
        });
    }
}
