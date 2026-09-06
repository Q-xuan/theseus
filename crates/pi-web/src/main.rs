use std::net::SocketAddr;

use pi_core::default_sessions_dir;
use pi_llm::ENV_API_KEY;

const DEFAULT_PORT: u16 = 43127;

#[tokio::main]
async fn main() {
    let port = std::env::var("PI_WEB_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let key_set = std::env::var(ENV_API_KEY)
        .ok()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    eprintln!("pi-web listening on http://{addr}");
    eprintln!("sessions: {}", default_sessions_dir().display());
    if !key_set {
        eprintln!("{ENV_API_KEY} is not set; turn/start will return an error until you export it and restart. The key is never shown in the UI.");
    }
    if let Err(err) = pi_web::serve(addr).await {
        eprintln!("pi-web failed: {err}");
        std::process::exit(1);
    }
}
