//! API key: show configured-or-not; optional paste writes to the user
//! environment / OS keychain only.
//!
//! Never written to `~/.theseus/`, the repo, jsonl, SessionEvent, WebView
//! storage, or Release assets. No OAuth.

use std::process::Command;

use crate::{key_is_set, ENV_API_KEY};

const MAX_KEY_CHARS: usize = 4096;
const KEYCHAIN_SERVICE: &str = "dev.theseus.desktop";
const KEYCHAIN_ACCOUNT: &str = "THESEUS_LLM_API_KEY";


pub fn normalize_key(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.chars().count() > MAX_KEY_CHARS {
        return None;
    }
    if s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    Some(s.to_string())
}

/// True if this process or a user-env / keychain store has a key.
pub fn key_configured() -> bool {
    if key_is_set() {
        return true;
    }
    os_key_present()
}

/// Load a stored user-env / keychain value into this process (not into UI).
pub fn hydrate_process_key() {
    if key_is_set() {
        return;
    }
    if let Some(key) = read_os_key() {
        std::env::set_var(ENV_API_KEY, key);
    }
}

/// Write to the current process and the user environment / OS keychain.
/// Never creates a file under `~/.theseus/`.
pub fn persist_user_key(raw: &str) -> Result<(), String> {
    let key = normalize_key(raw).ok_or_else(|| "invalid key".to_string())?;
    std::env::set_var(ENV_API_KEY, &key);
    persist_os_key(&key)?;
    debug_assert!(
        !key_written_to_theseus_dir(),
        "API key must not land in ~/.theseus"
    );
    Ok(())
}

fn key_written_to_theseus_dir() -> bool {
    let Some(home) = theseus_core::user_home_dir() else {
        return false;
    };
    let dir = home.join(".theseus");
    if !dir.is_dir() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().and_then(|n| n.to_str()) == Some("sessions") {
            continue;
        }
        if let Ok(body) = std::fs::read_to_string(&path) {
            if let Ok(env_key) = std::env::var(ENV_API_KEY) {
                if !env_key.is_empty() && body.contains(&env_key) {
                    return true;
                }
            }
        }
    }
    false
}

fn persist_os_key(key: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        persist_windows_user_env(key)
    }
    #[cfg(target_os = "macos")]
    {
        persist_macos(key)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        persist_linux(key)
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = key;
        Ok(())
    }
}

fn read_os_key() -> Option<String> {
    #[cfg(windows)]
    {
        read_windows_user_env()
    }
    #[cfg(target_os = "macos")]
    {
        read_macos_keychain()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        read_linux()
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

fn os_key_present() -> bool {
    read_os_key().is_some()
}

#[cfg(windows)]
fn persist_windows_user_env(key: &str) -> Result<(), String> {
    let status = Command::new("setx")
        .args([ENV_API_KEY, key])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("could not write user environment".into());
    }
    Ok(())
}

#[cfg(windows)]
fn read_windows_user_env() -> Option<String> {
    let out = Command::new("reg")
        .args([
            "query",
            r"HKCU\Environment",
            "/v",
            ENV_API_KEY,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if line.contains(ENV_API_KEY) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(val) = parts.last() {
                let s = val.trim();
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn persist_macos(key: &str) -> Result<(), String> {
    let _ = Command::new("launchctl")
        .args(["setenv", ENV_API_KEY, key])
        .status();
    let delete = Command::new("security")
        .args([
            "delete-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
        ])
        .output();
    let _ = delete;
    let status = Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
            key,
            "-U",
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("could not write keychain".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn read_macos_keychain() -> Option<String> {
    let out = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            KEYCHAIN_ACCOUNT,
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn persist_linux(key: &str) -> Result<(), String> {
    if Command::new("secret-tool").arg("--version").output().is_ok() {
        let mut child = Command::new("secret-tool")
            .args(["store", "--label=Theseus", "service", KEYCHAIN_SERVICE, "key", KEYCHAIN_ACCOUNT])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = writeln!(stdin, "{key}");
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        if status.success() {
            return Ok(());
        }
    }
    // systemd user environment (not the repo, not ~/.theseus, not jsonl).
    if let Some(home) = theseus_core::user_home_dir() {
        let dir = home.join(".config").join("environment.d");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("theseus-llm.conf");
        let body = format!("{ENV_API_KEY}={key}\n");
        std::fs::write(&path, body).map_err(|e| e.to_string())?;
        return Ok(());
    }
    Err("could not persist user environment".into())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn read_linux() -> Option<String> {
    if let Ok(out) = Command::new("secret-tool")
        .args(["lookup", "service", KEYCHAIN_SERVICE, "key", KEYCHAIN_ACCOUNT])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    let home = theseus_core::user_home_dir()?;
    let path = home.join(".config").join("environment.d").join("theseus-llm.conf");
    let body = std::fs::read_to_string(path).ok()?;
    for line in body.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(ENV_API_KEY) {
            let rest = rest.trim_start_matches('=').trim();
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    fn lock_env() -> MutexGuard<'static, ()> {
        crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn normalize_rejects_blank_and_controls() {
        assert_eq!(normalize_key("  "), None);
        assert_eq!(normalize_key("a b"), None);
        assert_eq!(normalize_key("sk-test"), Some("sk-test".into()));
    }

    #[test]
    fn persist_sets_process_env_not_theseus_file() {
        let _g = lock_env();
        let home = std::env::temp_dir().join(format!(
            "theseus-key-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(home.join(".theseus")).unwrap();
        let old_home = std::env::var("HOME").ok();
        let old_profile = std::env::var("USERPROFILE").ok();
        let old_key = std::env::var(ENV_API_KEY).ok();
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::remove_var(ENV_API_KEY);
        let token = format!("sk-test-{}", std::process::id());
        persist_user_key(&token).unwrap();
        assert_eq!(std::env::var(ENV_API_KEY).unwrap(), token);
        let theseus = home.join(".theseus");
        if let Ok(entries) = std::fs::read_dir(&theseus) {
            for entry in entries.flatten() {
                if let Ok(body) = std::fs::read_to_string(entry.path()) {
                    assert!(
                        !body.contains(&token),
                        "key leaked into {:?}",
                        entry.path()
                    );
                }
            }
        }
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match old_profile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        match old_key {
            Some(v) => std::env::set_var(ENV_API_KEY, v),
            None => std::env::remove_var(ENV_API_KEY),
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
