//! Thin desktop shell around the existing `theseus-app-server` **stdio** sidecar.
//!
//! The shell does not run `derive_messages`, the tool loop, or session I/O.
//! `THESEUS_LLM_API_KEY` (or temporary `PI_LLM_API_KEY`) is inherited by the
//! child if the parent exported it. A thin settings card can paste a key into
//! the **user environment / OS keychain only** — never into the repo, jsonl,
//! SessionEvent, or Release assets.
//!
//! The WebView / `--preview` page is a **client projection** (Thread / Turn /
//! Item + one approval card). Its chrome aims at Codex Desktop / dsh web
//! density — not a lock-in of the current `theseus-web/static` skin.

mod base_url;
mod bridge;
mod key;
mod locate;
mod model;
mod sidecar;
mod workspace;

pub use base_url::{
    persist_user_base_url, resolve_user_base_url, sidecar_base_url, DEFAULT_BASE_URL, ENV_BASE_URL,
    ENV_BASE_URL_LEGACY,
};
pub use bridge::{serve, serve_listener};
pub use key::{hydrate_process_key, key_configured, persist_user_key};
pub use locate::{bin_name, locate_app_server, LocateError};
pub use model::{
    persist_user_model, resolve_user_model, sidecar_model, DEFAULT_MODEL, ENV_MODEL,
    ENV_MODEL_LEGACY,
};
pub use sidecar::{Sidecar, SidecarError};
pub use workspace::{
    persist_user_workspace, pick_workspace_directory, resolve_user_workspace,
};

/// Default loopback port for `--preview` (uncommon; not a public service).
pub const DEFAULT_PREVIEW_PORT: u16 = 43173;

pub const ENV_SERVER_BIN: &str = "THESEUS_APP_SERVER_BIN";
pub const ENV_SERVER_BIN_LEGACY: &str = "PI_APP_SERVER_BIN";
pub const ENV_PREVIEW_PORT: &str = "THESEUS_DESKTOP_PORT";
pub const ENV_PREVIEW_PORT_LEGACY: &str = "PI_DESKTOP_PORT";
pub const ENV_API_KEY: &str = "THESEUS_LLM_API_KEY";
pub const ENV_API_KEY_LEGACY: &str = "PI_LLM_API_KEY";

pub fn key_is_set() -> bool {
    theseus_core::first_nonempty_env(&[ENV_API_KEY, ENV_API_KEY_LEGACY]).is_some()
}

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Product GUI (Release + `gui`, not `--preview`) stays quiet on stdio.
/// Debug builds and `--preview` may still print locate / session / bridge lines.
pub fn verbose_stdio() -> bool {
    cfg!(debug_assertions) || std::env::args().any(|a| a == "--preview")
}

#[cfg(feature = "gui")]
mod gui;

#[cfg(feature = "gui")]
pub use gui::run_gui;
