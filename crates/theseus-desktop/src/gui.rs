use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder};

use crate::sidecar::Sidecar;
use crate::{key_is_set, ENV_API_KEY};

/// Open a native window onto the loopback client. Sidecar stays on stdio.
pub fn run_gui() {
    let (sidecar, rx) = match Sidecar::start() {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("theseus-desktop: {err}");
            std::process::exit(1);
        }
    };
    if crate::verbose_stdio() {
        eprintln!("theseus-desktop sidecar: {}", sidecar.path().display());
        eprintln!(
            "sessions: {}",
            theseus_core::default_sessions_dir().display()
        );
        if !key_is_set() {
            eprintln!(
                "{ENV_API_KEY} is not set on this process; the sidecar inherits that. \
                 turn/start will error until you export it and restart. The key is never shown in the UI."
            );
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio");
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local_addr");
    let url = format!("http://{addr}/");
    if crate::verbose_stdio() {
        eprintln!("theseus-desktop UI on {url} (127.0.0.1 only; stdio↔WS bridge is transitional)");
    }

    let serve_sidecar = sidecar.clone();
    std::thread::Builder::new()
        .name("theseus-desktop-bridge".into())
        .spawn(move || {
            runtime.block_on(async move {
                if let Err(err) = crate::bridge::serve_listener(listener, serve_sidecar, rx).await {
                    eprintln!("theseus-desktop bridge: {err}");
                }
            });
        })
        .expect("bridge thread");

    let exit_sidecar = sidecar.clone();
    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?))
                .title("Theseus")
                .inner_size(1120.0, 780.0)
                .min_inner_size(760.0, 520.0)
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("tauri")
        .run(move |_app, event| {
            if matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
                exit_sidecar.shutdown();
            }
            let _ = _app;
        });
    sidecar.shutdown();
}
