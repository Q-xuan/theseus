//! Spawn the stdio binary. Without PI_LLM_API_KEY, turn/start must fail cleanly.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

#[test]
fn stdio_thread_start_then_turn_start_without_key() {
    let sess = std::env::temp_dir().join(format!(
        "pi-smoke-sess-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&sess).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_pi-app-server"))
        .env_remove("PI_LLM_API_KEY")
        .env("PI_SESSIONS_DIR", &sess)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn pi-app-server");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"clientInfo":{{"name":"smoke","version":"0.1.0"}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"thread/start","params":{{}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();

    let mut thread_id = None;
    let mut lines = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while thread_id.is_none() && std::time::Instant::now() < deadline {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        if v.get("id") == Some(&Value::from(1)) {
            thread_id = v["result"]["thread"]["id"].as_str().map(str::to_string);
        }
        lines.push(v);
    }
    let thread_id = thread_id.expect("thread/start result");
    let started = lines
        .iter()
        .find(|v| v.get("id") == Some(&Value::from(1)))
        .expect("thread/start line");
    let path = started["result"]["thread"]["path"]
        .as_str()
        .expect("Thread.path");
    assert!(
        path.ends_with(&format!("{thread_id}.jsonl")),
        "{path}"
    );
    assert!(std::path::Path::new(path).is_file());
    let jsonl = std::fs::read_to_string(path).unwrap();
    assert!(jsonl.contains("thread/meta"));
    assert!(!jsonl.contains("assistant/chunk"));
    assert_eq!(started["result"]["thread"]["ephemeral"], false);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"turn/start","params":{{"threadId":"{thread_id}","input":[{{"type":"text","text":"hello"}}]}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":3,"method":"shutdown","params":{{}}}}"#
    )
    .unwrap();
    drop(stdin);

    let mut saw_turn_rpc_error = false;
    let mut saw_assistant_in_history = false;
    let mut saw_turn_start_event = false;

    for line in reader.lines() {
        let line = line.unwrap();
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line).expect("json line");
        if v.get("id") == Some(&Value::from(2)) {
            let msg = v["error"]["message"].as_str().unwrap_or("");
            assert!(
                msg.contains("PI_LLM_API_KEY"),
                "expected missing-key error, got {v}"
            );
            saw_turn_rpc_error = true;
        }
        if v.get("method") == Some(&Value::from("session/event")) {
            let ty = v["params"]["event"]["type"].as_str().unwrap_or("");
            if ty == "turn/start" {
                saw_turn_start_event = true;
            }
            if ty == "assistant/message" {
                saw_assistant_in_history = true;
            }
        }
        lines.push(v);
    }

    let status = child.wait().expect("wait");
    assert!(status.success(), "server exited {status}");
    assert!(
        saw_turn_rpc_error,
        "missing turn/start RPC error: {lines:?}"
    );
    assert!(
        !saw_turn_start_event,
        "no-key turn must not append turn/start"
    );
    assert!(
        !saw_assistant_in_history,
        "no-key turn must not append assistant/message"
    );

    let jsonl = std::fs::read_to_string(sess.join(format!("{thread_id}.jsonl"))).unwrap();
    assert_eq!(jsonl.lines().count(), 1, "no-key turn must not grow the log");
    let _ = std::fs::remove_dir_all(sess);
}
