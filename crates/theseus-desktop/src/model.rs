//! Composer model string for the desktop shell.
//!
//! Read order matches `theseus-llm`: `THESEUS_LLM_MODEL`, then short-lived
//! `PI_LLM_MODEL`, then `~/.theseus/model`, then [`DEFAULT_MODEL`].
//! Writing persists the name under `~/.theseus/` (not a secret, not the repo)
//! and sets `THESEUS_LLM_MODEL` on this process so the next `thread/start`
//! and a later sidecar spawn see the same string.

use std::fs;
use std::path::PathBuf;

use theseus_core::user_home_dir;

pub const ENV_MODEL: &str = "THESEUS_LLM_MODEL";
pub const ENV_MODEL_LEGACY: &str = "PI_LLM_MODEL";
pub const DEFAULT_MODEL: &str = "gpt-4o-mini";

const CONFIG_DIR_NAME: &str = ".theseus";
const MODEL_FILE_NAME: &str = "model";
const MAX_MODEL_CHARS: usize = 128;


fn first_nonempty_env(names: &[&str]) -> Option<String> {
    theseus_core::first_nonempty_env(names).map(|s| s.trim().to_string())
}

/// `~/.theseus/model` — user home via [`user_home_dir`] (`HOME` / `USERPROFILE`).
pub fn user_model_path() -> Option<PathBuf> {
    Some(
        user_home_dir()?
            .join(CONFIG_DIR_NAME)
            .join(MODEL_FILE_NAME),
    )
}

fn read_model_file() -> Option<String> {
    let path = user_model_path()?;
    let raw = fs::read_to_string(path).ok()?;
    normalize_model(&raw)
}

/// Trim, reject empty / control / path-escape names.
pub fn normalize_model(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.chars().count() > MAX_MODEL_CHARS {
        return None;
    }
    if s.chars().any(|c| c.is_control() || c == '\\') {
        return None;
    }
    if s.split('/').any(|part| part == "..") {
        return None;
    }
    Some(s.to_string())
}

/// Env → legacy env → `~/.theseus/model` → default.
pub fn resolve_user_model() -> String {
    first_nonempty_env(&[ENV_MODEL, ENV_MODEL_LEGACY])
        .and_then(|s| normalize_model(&s))
        .or_else(read_model_file)
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

/// Persist a model name. Updates `THESEUS_LLM_MODEL` in this process.
pub fn persist_user_model(raw: &str) -> Result<String, String> {
    let model = normalize_model(raw).ok_or_else(|| "invalid model name".to_string())?;
    let path = user_model_path().ok_or_else(|| "cannot resolve home directory".to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&path, format!("{model}\n")).map_err(|e| e.to_string())?;
    std::env::set_var(ENV_MODEL, &model);
    Ok(model)
}

/// Value to inject as `THESEUS_LLM_MODEL` on the sidecar child.
pub fn sidecar_model() -> String {
    resolve_user_model()
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
            "theseus-model-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&home).unwrap();
        let old_home = std::env::var("HOME").ok();
        let old_profile = std::env::var("USERPROFILE").ok();
        let old_model = std::env::var(ENV_MODEL).ok();
        let old_legacy = std::env::var(ENV_MODEL_LEGACY).ok();
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::remove_var(ENV_MODEL);
        std::env::remove_var(ENV_MODEL_LEGACY);
        f(&home);
        match old_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match old_profile {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        match old_model {
            Some(v) => std::env::set_var(ENV_MODEL, v),
            None => std::env::remove_var(ENV_MODEL),
        }
        match old_legacy {
            Some(v) => std::env::set_var(ENV_MODEL_LEGACY, v),
            None => std::env::remove_var(ENV_MODEL_LEGACY),
        }
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn normalize_rejects_blank_and_controls() {
        assert_eq!(normalize_model("  "), None);
        assert_eq!(normalize_model("a\nb"), None);
        assert_eq!(normalize_model("../x"), None);
        assert_eq!(normalize_model("gpt-4o-mini"), Some("gpt-4o-mini".into()));
        assert_eq!(
            normalize_model("  org/gpt-4o  "),
            Some("org/gpt-4o".into())
        );
    }

    #[test]
    fn file_round_trip_and_default() {
        with_temp_home(|home| {
            assert_eq!(resolve_user_model(), DEFAULT_MODEL);
            let got = persist_user_model("gpt-4o").unwrap();
            assert_eq!(got, "gpt-4o");
            let path = home.join(".theseus").join("model");
            assert_eq!(fs::read_to_string(path).unwrap().trim(), "gpt-4o");
            assert_eq!(std::env::var(ENV_MODEL).unwrap(), "gpt-4o");
            std::env::remove_var(ENV_MODEL);
            assert_eq!(resolve_user_model(), "gpt-4o");
        });
    }

    #[test]
    fn theseus_env_wins_then_legacy_pi() {
        with_temp_home(|_| {
            persist_user_model("from-file").unwrap();
            std::env::remove_var(ENV_MODEL);
            std::env::set_var(ENV_MODEL_LEGACY, "from-pi");
            assert_eq!(resolve_user_model(), "from-pi");
            std::env::set_var(ENV_MODEL, "from-theseus");
            assert_eq!(resolve_user_model(), "from-theseus");
        });
    }
}
