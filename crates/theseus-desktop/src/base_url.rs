//! LLM base URL for the desktop shell.
//!
//! Read order matches `theseus-llm`: `THESEUS_LLM_BASE_URL`, then short-lived
//! `PI_LLM_BASE_URL`, then `~/.theseus/base_url`, then [`DEFAULT_BASE_URL`].
//! Writing persists under `~/.theseus/` (not a secret, not the repo) and sets
//! `THESEUS_LLM_BASE_URL` on this process so the next sidecar spawn sees it.

use std::fs;
use std::path::PathBuf;

use theseus_core::user_home_dir;

pub const ENV_BASE_URL: &str = "THESEUS_LLM_BASE_URL";
pub const ENV_BASE_URL_LEGACY: &str = "PI_LLM_BASE_URL";
pub const DEFAULT_BASE_URL: &str = "https://ai.aruyx.com/";

const CONFIG_DIR_NAME: &str = ".theseus";
const BASE_URL_FILE_NAME: &str = "base_url";
const MAX_BASE_URL_CHARS: usize = 512;


fn first_nonempty_env(names: &[&str]) -> Option<String> {
    theseus_core::first_nonempty_env(names).map(|s| s.trim().to_string())
}

pub fn user_base_url_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(CONFIG_DIR_NAME)
            .join(BASE_URL_FILE_NAME),
    )
}

fn read_base_url_file() -> Option<String> {
    let path = user_base_url_path()?;
    let raw = fs::read_to_string(path).ok()?;
    normalize_base_url(&raw)
}

/// Trim, require http(s), reject empty / control / oversized values.
pub fn normalize_base_url(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.chars().count() > MAX_BASE_URL_CHARS {
        return None;
    }
    if s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return None;
    }
    Some(s.to_string())
}

/// Env → legacy env → `~/.theseus/base_url` → default.
pub fn resolve_user_base_url() -> String {
    first_nonempty_env(&[ENV_BASE_URL, ENV_BASE_URL_LEGACY])
        .and_then(|s| normalize_base_url(&s))
        .or_else(read_base_url_file)
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// Persist a base URL. Updates `THESEUS_LLM_BASE_URL` in this process.
pub fn persist_user_base_url(raw: &str) -> Result<String, String> {
    let url = normalize_base_url(raw).ok_or_else(|| "invalid base url".to_string())?;
    let path = user_base_url_path().ok_or_else(|| "cannot resolve home directory".to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&path, format!("{url}\n")).map_err(|e| e.to_string())?;
    std::env::set_var(ENV_BASE_URL, &url);
    Ok(url)
}

/// Value to inject as `THESEUS_LLM_BASE_URL` on the sidecar child.
pub fn sidecar_base_url() -> String {
    resolve_user_base_url()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    fn lock_env() -> MutexGuard<'static, ()> {
        crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with_temp_home<F: FnOnce(&std::path::Path)>(f: F) {
        let _g = lock_env();
        let home = std::env::temp_dir().join(format!(
            "theseus-base-url-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&home).unwrap();
        let old_home = std::env::var("HOME").ok();
        let old_profile = std::env::var("USERPROFILE").ok();
        let old_url = std::env::var(ENV_BASE_URL).ok();
        let old_legacy = std::env::var(ENV_BASE_URL_LEGACY).ok();
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::remove_var(ENV_BASE_URL);
        std::env::remove_var(ENV_BASE_URL_LEGACY);
        f(&home);
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match old_profile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        match old_url {
            Some(v) => std::env::set_var(ENV_BASE_URL, v),
            None => std::env::remove_var(ENV_BASE_URL),
        }
        match old_legacy {
            Some(v) => std::env::set_var(ENV_BASE_URL_LEGACY, v),
            None => std::env::remove_var(ENV_BASE_URL_LEGACY),
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn normalize_rejects_blank_and_non_http() {
        assert_eq!(normalize_base_url("  "), None);
        assert_eq!(normalize_base_url("ftp://x"), None);
        assert_eq!(normalize_base_url("https://ex ample"), None);
        assert_eq!(
            normalize_base_url("  https://ai.aruyx.com/  "),
            Some("https://ai.aruyx.com/".into())
        );
        assert_eq!(
            normalize_base_url("http://127.0.0.1:8080/v1"),
            Some("http://127.0.0.1:8080/v1".into())
        );
    }

    #[test]
    fn file_round_trip_and_default() {
        with_temp_home(|home| {
            assert_eq!(resolve_user_base_url(), DEFAULT_BASE_URL);
            let got = persist_user_base_url("https://example.test/").unwrap();
            assert_eq!(got, "https://example.test/");
            let path = home.join(".theseus").join("base_url");
            assert_eq!(fs::read_to_string(path).unwrap().trim(), "https://example.test/");
            assert_eq!(std::env::var(ENV_BASE_URL).unwrap(), "https://example.test/");
            std::env::remove_var(ENV_BASE_URL);
            assert_eq!(resolve_user_base_url(), "https://example.test/");
        });
    }

    #[test]
    fn theseus_env_wins_then_legacy_pi() {
        with_temp_home(|_| {
            persist_user_base_url("https://from-file.test/").unwrap();
            std::env::remove_var(ENV_BASE_URL);
            std::env::set_var(ENV_BASE_URL_LEGACY, "https://from-pi.test/");
            assert_eq!(resolve_user_base_url(), "https://from-pi.test/");
            std::env::set_var(ENV_BASE_URL, "https://from-theseus.test/");
            assert_eq!(resolve_user_base_url(), "https://from-theseus.test/");
        });
    }
}
