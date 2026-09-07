use std::net::SocketAddr;

use theseus_core::{default_sessions_dir, first_nonempty_env};
use theseus_llm::{ENV_API_KEY, ENV_API_KEY_LEGACY};

const DEFAULT_PORT: u16 = 43127;

#[tokio::main]
async fn main() {
    let port = first_nonempty_env(&["THESEUS_WEB_PORT", "PI_WEB_PORT"])
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let key_set = first_nonempty_env(&[ENV_API_KEY, ENV_API_KEY_LEGACY]).is_some();
    eprintln!("theseus-web listening on http://{addr}");
    eprintln!("sessions: {}", default_sessions_dir().display());
    if !key_set {
        eprintln!("{ENV_API_KEY} is not set; turn/start will return an error until you export it and restart. The key is never shown in the UI.");
    }
    if let Err(err) = theseus_web::serve(addr).await {
        eprintln!("theseus-web failed: {err}");
        std::process::exit(1);
    }
}
