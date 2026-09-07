//! Thin desktop shell around the existing `theseus-app-server` **stdio** sidecar.
//!
//! The shell does not run `derive_messages`, the tool loop, or session I/O.
//! `THESEUS_LLM_API_KEY` (or temporary `PI_LLM_API_KEY`) is inherited by the
//! child if the parent exported it; it is never read into UI, config files, or
//! command-line flags.
//!
//! The WebView / `--preview` page is a **client projection** (Thread / Turn /
//! Item + one approval card). Its chrome aims at Codex Desktop / dsh web
//! density — not a lock-in of the current `theseus-web/static` skin.

mod bridge;
mod locate;
mod sidecar;

pub use bridge::{serve, serve_listener};
pub use locate::{bin_name, locate_app_server, LocateError};
pub use sidecar::{Sidecar, SidecarError};

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

#[cfg(feature = "gui")]
mod gui;

#[cfg(feature = "gui")]
pub use gui::run_gui;
