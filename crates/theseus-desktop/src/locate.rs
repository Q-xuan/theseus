use std::env;
use std::path::{Path, PathBuf};

use crate::{ENV_SERVER_BIN, ENV_SERVER_BIN_LEGACY};

const TARGET_TRIPLE: &str = env!("THESEUS_TARGET_TRIPLE");

#[derive(Debug, thiserror::Error)]
pub enum LocateError {
    #[error("{ENV_SERVER_BIN}={path} is not a file")]
    EnvNotAFile { path: String },
    #[error(
        "theseus-app-server binary not found. A packaged app should ship it next to \
         this executable. For a source tree: `cargo build -p theseus-app-server` \
         or set {ENV_SERVER_BIN}."
    )]
    NotFound,
}

pub fn bin_name() -> &'static str {
    if cfg!(windows) {
        "theseus-app-server.exe"
    } else {
        "theseus-app-server"
    }
}

/// Names Tauri may copy into the bundle (plain, or still triple-suffixed).
pub fn sidecar_file_names() -> Vec<String> {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    vec![
        bin_name().to_string(),
        format!("theseus-app-server-{TARGET_TRIPLE}{suffix}"),
    ]
}

/// Directories that may hold the embedded sidecar after `tauri build`.
pub fn bundle_sidecar_dirs(exe_dir: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![exe_dir.to_path_buf()];
    dirs.push(exe_dir.join("resources"));
    dirs.push(exe_dir.join("binaries"));
    if exe_dir.file_name().and_then(|n| n.to_str()) == Some("MacOS") {
        if let Some(contents) = exe_dir.parent() {
            dirs.push(contents.join("MacOS"));
            dirs.push(contents.join("Resources"));
            dirs.push(contents.join("Resources").join("binaries"));
        }
    }
    dirs
}

pub fn first_existing(dirs: &[PathBuf], names: &[String]) -> Option<PathBuf> {
    for dir in dirs {
        for name in names {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Resolve the sidecar binary. Never consults UI or a settings file.
pub fn locate_app_server() -> Result<PathBuf, LocateError> {
    if let Some(raw) = theseus_core::first_nonempty_env(&[ENV_SERVER_BIN, ENV_SERVER_BIN_LEGACY]) {
        let path = PathBuf::from(&raw);
        if path.is_file() {
            return Ok(path);
        }
        return Err(LocateError::EnvNotAFile { path: raw });
    }

    let names = sidecar_file_names();
    let mut dirs = Vec::new();

    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.extend(bundle_sidecar_dirs(dir));
        }
    }

    dirs.extend(target_dirs());

    first_existing(&dirs, &names).ok_or(LocateError::NotFound)
}

fn target_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(ws) = manifest.parent().and_then(|p| p.parent()) {
        push_profile_targets(&mut dirs, ws);
    }
    if let Ok(cwd) = env::current_dir() {
        for anc in cwd.ancestors() {
            push_profile_targets(&mut dirs, anc);
            if anc.join("Cargo.toml").is_file() && anc.join("crates").is_dir() {
                break;
            }
        }
    }
    dirs
}

fn push_profile_targets(dirs: &mut Vec<PathBuf>, root: &Path) {
    dirs.push(root.join("target/debug"));
    dirs.push(root.join("target/release"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bin_name_matches_platform() {
        let name = bin_name();
        assert!(name.starts_with("theseus-app-server"));
        if cfg!(windows) {
            assert!(name.ends_with(".exe"));
        }
    }

    #[test]
    fn sidecar_names_include_plain_and_triple() {
        let names = sidecar_file_names();
        assert!(names.iter().any(|n| n == bin_name()));
        assert!(names.iter().any(|n| n.contains(TARGET_TRIPLE)));
        assert!(!TARGET_TRIPLE.is_empty());
        assert_ne!(TARGET_TRIPLE, "unknown");
    }

    #[test]
    fn macos_bundle_dirs_include_contents_macos_and_resources() {
        let macos = PathBuf::from("/Applications/Theseus.app/Contents/MacOS");
        let dirs = bundle_sidecar_dirs(&macos);
        assert!(dirs.iter().any(|d| d.ends_with("MacOS")));
        assert!(dirs.iter().any(|d| d.ends_with("Resources")));
    }

    #[test]
    fn first_existing_prefers_plain_name_beside_exe() {
        let root = env::temp_dir().join(format!("theseus-locate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let bin = root.join(bin_name());
        std::fs::write(&bin, b"sidecar").unwrap();
        let found = first_existing(&[root.clone()], &sidecar_file_names()).unwrap();
        assert_eq!(found, bin);
        let _ = std::fs::remove_dir_all(&root);
    }
}
