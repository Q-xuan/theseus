//! Sidecar is a real `pi --mode rpc` process (or the test stand-in). Shutdown must not leave orphans.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;
use theseus_desktop::Sidecar;

fn fake_pi_bin() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fake_pi.py");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&path).expect("fake_pi.py").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).unwrap();
    }
    path
}

fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

#[test]
fn spawn_get_state_new_session_then_shutdown_reaps_child() {
    let bin = fake_pi_bin();
    let (sidecar, rx) = Sidecar::spawn(&bin).expect("spawn sidecar");
    let pid = sidecar.pid();
    assert!(
        process_exists(pid),
        "sidecar should be alive after get_state handshake"
    );

    sidecar
        .send_line(r#"{"id":1,"type":"new_session"}"#)
        .unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut found = None;
    while found.is_none() && std::time::Instant::now() < deadline {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(200)) {
            assert!(
                !line.contains("sk-"),
                "bridge must not echo secrets: {line}"
            );
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                if v.get("id") == Some(&Value::from(1)) && v.get("command") == Some(&Value::from("new_session")) {
                    found = Some(v);
                }
            }
        }
    }
    let v = found.expect("new_session");
    assert_eq!(v["success"], true);

    sidecar.shutdown();
    let gone_deadline = std::time::Instant::now() + Duration::from_secs(3);
    while process_exists(pid) && std::time::Instant::now() < gone_deadline {
        std::thread::sleep(Duration::from_millis(40));
    }
    assert!(
        !process_exists(pid),
        "sidecar pid {pid} still alive after shutdown"
    );
}

#[tokio::test]
async fn preview_health_does_not_mention_secrets() {
    let bin = fake_pi_bin();
    let (sidecar, rx) = Sidecar::spawn(&bin).expect("spawn");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_sidecar = sidecar.clone();
    tokio::spawn(async move {
        let _ = theseus_desktop::serve_listener(listener, serve_sidecar, rx).await;
    });

    let body = tokio::task::spawn_blocking(move || {
        let mut last = String::new();
        for _ in 0..40 {
            if let Ok(mut stream) = TcpStream::connect(addr) {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let req = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
                if stream.write_all(req.as_bytes()).is_ok() {
                    last.clear();
                    let _ = stream.read_to_string(&mut last);
                    if last.contains("theseus-desktop") {
                        return last;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        last
    })
    .await
    .unwrap();

    assert!(body.contains(r#""ok":true"#), "{body}");
    assert!(body.contains("pi-rpc"), "{body}");
    assert!(!body.contains("sk-"));
    assert!(!body.contains("THESEUS_LLM_API_KEY"));

    let model_body = tokio::task::spawn_blocking(move || {
        let mut last = String::new();
        if let Ok(mut stream) = TcpStream::connect(addr) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let req = "GET /model HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
            if stream.write_all(req.as_bytes()).is_ok() {
                last.clear();
                let _ = stream.read_to_string(&mut last);
            }
        }
        last
    })
    .await
    .unwrap();
    assert!(model_body.contains("\"model\""), "{model_body}");
    assert!(!model_body.contains("THESEUS_LLM_API_KEY"));
    assert!(!model_body.contains("sk-"));

    let settings_body = tokio::task::spawn_blocking(move || {
        let mut last = String::new();
        if let Ok(mut stream) = TcpStream::connect(addr) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let req = "GET /settings HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
            if stream.write_all(req.as_bytes()).is_ok() {
                last.clear();
                let _ = stream.read_to_string(&mut last);
            }
        }
        last
    })
    .await
    .unwrap();
    assert!(settings_body.contains("\"workspace\""), "{settings_body}");
    assert!(settings_body.contains("\"provider\""), "{settings_body}");
    assert!(!settings_body.contains("THESEUS_LLM_API_KEY"));
    assert!(!settings_body.contains("sk-"));
    sidecar.shutdown();
}
