use std::env;
use std::path::{Path, PathBuf};

use crate::{ENV_PI_BIN, ENV_PI_BIN_LEGACY};

#[derive(Debug, thiserror::Error)]
pub enum LocateError {
    #[error("{ENV_PI_BIN}={path} is not a file")]
    EnvNotAFile { path: String },
    #[error(
        "pi not found on PATH. Theseus desktop talks to `pi --mode rpc` \
         (https://github.com/badlogic/pi-mono). Install `pi` and put it on PATH \
         so GUI apps can see it (Windows: not only your shell). \
         Override with {ENV_PI_BIN}=/path/to/pi"
    )]
    NotFound,
}

/// `pi` on this platform (`pi.exe` / `pi.cmd` still match via PATH search).
pub fn bin_name() -> &'static str {
    if cfg!(windows) {
        "pi.exe"
    } else {
        "pi"
    }
}

fn candidate_names() -> Vec<String> {
    let mut names = vec!["pi".to_string()];
    if cfg!(windows) {
        names.push("pi.exe".into());
        names.push("pi.cmd".into());
        names.push("pi.bat".into());
    }
    names
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            return meta.permissions().mode() & 0o111 != 0;
        }
    }
    true
}

/// Walk PATH (and Windows PATHEXT) for `pi`.
pub fn search_path() -> Option<PathBuf> {
    let path_os = env::var_os("PATH")?;
    let names = candidate_names();
    for dir in env::split_paths(&path_os) {
        for name in &names {
            let candidate = dir.join(name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolve the `pi` binary. Never consults UI or a settings file.
pub fn locate_pi() -> Result<PathBuf, LocateError> {
    if let Some(raw) = theseus_core::first_nonempty_env(&[ENV_PI_BIN, ENV_PI_BIN_LEGACY]) {
        let path = PathBuf::from(&raw);
        if path.is_file() {
            return Ok(path);
        }
        return Err(LocateError::EnvNotAFile { path: raw });
    }
    search_path().ok_or(LocateError::NotFound)
}

/// Back-compat alias used by older tests / docs that said “app server”.
pub fn locate_app_server() -> Result<PathBuf, LocateError> {
    locate_pi()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_name_is_pi() {
        let name = bin_name();
        assert!(name.starts_with("pi"));
        if cfg!(windows) {
            assert!(name.ends_with(".exe"));
        }
    }

    #[test]
    fn missing_env_file_is_a_clear_error() {
        let err = LocateError::EnvNotAFile {
            path: "/no/such/pi".into(),
        };
        let text = err.to_string();
        assert!(text.contains(ENV_PI_BIN));
        assert!(text.contains("/no/such/pi"));
    }

    #[test]
    fn not_found_mentions_path_and_repo() {
        let text = LocateError::NotFound.to_string();
        assert!(text.contains("PATH"));
        assert!(text.contains("pi --mode rpc"));
        assert!(text.contains("badlogic/pi-mono"));
        assert!(text.contains(ENV_PI_BIN));
    }
}
