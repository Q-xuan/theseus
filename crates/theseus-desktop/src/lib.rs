//! Thin desktop shell around `pi --mode rpc`.
//!
//! The shell projects Thread / Turn / Item and sends intents
//! (`new_session`, `prompt`, `abort`, …). It does not run `derive_messages`,
//! the tool loop, or a parallel `theseus-app-server` product path.
//! Keys and providers stay inside [pi](https://github.com/badlogic/pi-mono).

mod base_url;
mod bridge;
mod jsonl;
mod key;
mod locate;
mod map;
mod model;
mod sessions;
mod sidecar;
mod workspace;

pub use base_url::{
    persist_user_base_url, resolve_user_base_url, sidecar_base_url, DEFAULT_BASE_URL, ENV_BASE_URL,
    ENV_BASE_URL_LEGACY,
};
pub use bridge::{serve, serve_listener};
pub use jsonl::{drain_jsonl_lines, read_jsonl};
pub use key::{hydrate_process_key, key_configured, persist_user_key};
pub use locate::{bin_name, locate_app_server, locate_pi, LocateError};
pub use map::{enrich_result, is_pi_command, is_theseus_app_server_method, project_messages, EventMap};
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

pub const ENV_PI_BIN: &str = "THESEUS_PI_BIN";
pub const ENV_PI_BIN_LEGACY: &str = "PI_BIN";
/// @deprecated product path is PATH `pi`; kept so old help text still compiles.
pub const ENV_SERVER_BIN: &str = ENV_PI_BIN;
pub const ENV_SERVER_BIN_LEGACY: &str = ENV_PI_BIN_LEGACY;
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
