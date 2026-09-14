#![cfg_attr(
    all(windows, feature = "gui", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use std::net::SocketAddr;

use theseus_desktop::{Sidecar, DEFAULT_PREVIEW_PORT, ENV_PI_BIN, ENV_PREVIEW_PORT, ENV_PREVIEW_PORT_LEGACY};

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--help") | Some("-h") => print_help(),
        Some("--preview") => run_preview(),
        Some("--gui") => run_gui_or_hint(),
        Some(other) => {
            eprintln!("unknown argument: {other}");
            print_help();
            std::process::exit(2);
        }
        None => {
            #[cfg(feature = "gui")]
            theseus_desktop::run_gui();
            #[cfg(not(feature = "gui"))]
            run_preview();
        }
    }
}

fn print_help() {
    eprintln!(
        "\
theseus-desktop — Tauri thin shell over `pi --mode rpc`

Usage:
  theseus-desktop              macOS/Windows (gui feature): native window
                          Linux / no gui: loopback preview
  theseus-desktop --preview    127.0.0.1 client (stdio↔WS, transitional)
  theseus-desktop --gui        native window (requires --features gui)
  theseus-desktop --help

The sidecar is `pi --mode rpc` (JSONL). Locate order:
  1. ${bin} (explicit path)
  2. `pi` on PATH

Windows needs `pi` on PATH (https://github.com/badlogic/pi-mono).
Release GUI uses the windows subsystem and CREATE_NO_WINDOW — no black console.

Keys and providers stay inside pi. This shell does not write ~/.theseus key
files and does not speak the theseus-app-server protocol on the product path.

Preview port: ${port} or {default} (127.0.0.1 only).
",
        bin = ENV_PI_BIN,
        port = ENV_PREVIEW_PORT,
        default = DEFAULT_PREVIEW_PORT,
    );
}

fn run_gui_or_hint() {
    #[cfg(feature = "gui")]
    {
        theseus_desktop::run_gui();
    }
    #[cfg(not(feature = "gui"))]
    {
        eprintln!("theseus-desktop was built without `gui`. On macOS/Windows:");
        eprintln!("  cargo run -p theseus-desktop --features gui");
        eprintln!("  # or: cargo tauri dev   (after `cargo install tauri-cli --version '^2.0'`)");
        std::process::exit(2);
    }
}

fn run_preview() {
    let port = theseus_core::first_nonempty_env(&[ENV_PREVIEW_PORT, ENV_PREVIEW_PORT_LEGACY])
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PREVIEW_PORT);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let (sidecar, rx) = match Sidecar::start() {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("theseus-desktop: {err}");
            std::process::exit(1);
        }
    };

    eprintln!("theseus-desktop sidecar: {} --mode rpc", sidecar.path().display());
    eprintln!(
        "theseus-desktop preview http://{addr}  (127.0.0.1 only; stdio↔WS bridge is transitional)"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio");
    let serve_sidecar = sidecar.clone();
    runtime.block_on(async move {
        let server = theseus_desktop::serve(addr, serve_sidecar, rx);
        tokio::select! {
            result = server => {
                if let Err(err) = result {
                    eprintln!("theseus-desktop preview failed: {err}");
                }
            }
            _ = tokio::signal::ctrl_c() => {}
        }
    });
    sidecar.shutdown();
}
