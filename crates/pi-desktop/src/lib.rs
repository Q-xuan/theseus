//! Thin desktop shell around the existing `pi-app-server` **stdio** sidecar.
//!
//! The shell does not run `derive_messages`, the tool loop, or session I/O.
//! `PI_LLM_API_KEY` is inherited by the child if the parent exported it; it is
//! never read into UI, config files, or command-line flags.
//!
//! The WebView / `--preview` page is a **client projection** (Thread / Turn /
//! Item + one approval card). Its chrome aims at Codex Desktop / dsh web
//! density — not a lock-in of the current `pi-web/static` skin.

mod bridge;
mod locate;
mod sidecar;

pub use bridge::{serve, serve_listener};
pub use locate::{bin_name, locate_app_server, LocateError};
pub use sidecar::{Sidecar, SidecarError};

/// Default loopback port for `--preview` (uncommon; not a public service).
pub const DEFAULT_PREVIEW_PORT: u16 = 43173;

pub const ENV_SERVER_BIN: &str = "PI_APP_SERVER_BIN";
pub const ENV_PREVIEW_PORT: &str = "PI_DESKTOP_PORT";
pub const ENV_API_KEY: &str = "PI_LLM_API_KEY";

pub fn key_is_set() -> bool {
    std::env::var(ENV_API_KEY)
        .ok()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(feature = "gui")]
mod gui;

#[cfg(feature = "gui")]
pub use gui::run_gui;
