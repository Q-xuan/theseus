//! Desktop chrome is a client projection. It must not grow a loop or a key box.

#[test]
fn desktop_ui_stays_a_client() {
    let html = include_str!("../ui/index.html");
    let js = include_str!("../ui/app.js");
    let css = include_str!("../ui/app.css");
    for hay in [html, js, css] {
        assert!(!hay.contains("PI_LLM_API_KEY"));
        assert!(!hay.contains("apiKey"));
        assert!(!hay.contains("api_key"));
        assert!(!hay.contains("password"));
        assert!(!hay.contains("derive_messages"));
        assert!(!hay.contains("MCP"));
        assert!(!hay.contains("compaction"));
        assert!(!hay.contains("PI_SESSIONS_DIR"));
        assert!(!hay.contains("PI_HOME"));
        assert!(!hay.contains(".pi-app"));
        assert!(!hay.contains("PI_TOOL_APPROVAL"));
    }
    assert!(html.contains("data-app=\"pi-desktop\""));
    assert!(html.contains("id=\"sessions\""), "sidebar session list");
    assert!(html.contains("最近"), "recency list label");
    assert!(html.contains("id=\"approval\""));
    assert!(html.contains("id=\"dock\""), "composer dock for the one gate");
    assert!(html.contains("新对话"));
    assert!(!html.to_lowercase().contains("settings"));
    assert!(!html.contains("type=\"search\""));
    assert!(js.contains("item/agentMessage/delta"));
    assert!(js.contains("thread/start"));
    assert!(js.contains("thread/resume"));
    assert!(js.contains("thread/list"));
    assert!(js.contains("turn/start"));
    assert!(js.contains("tool/approve"));
    assert!(js.contains("tool/reject"));
    assert!(js.contains("item/tool/approval/request"));
    assert!(js.contains("readableError"));
    assert!(js.contains("turn-mark"));
    assert!(js.contains("tool-head"));
    assert!(
        !js.contains("session/event"),
        "UI must project Item/delta, not raw session log"
    );
    assert!(
        !html.contains("data-app=\"pi-web\""),
        "desktop chrome is not the pi-web skin"
    );
}

#[test]
fn chrome_cites_dsh_modules_not_a_pixel_clone() {
    let css = include_str!("../ui/app.css");
    assert!(css.contains("SidebarRoot.module.css"));
    assert!(css.contains("ApprovalPanel"));
    assert!(css.contains("GenericToolCard"));
    assert!(css.contains("38px"), "dsh new-session bar height");
    assert!(css.contains("gate-strip"), "amber approval strip");
    let html = include_str!("../ui/index.html");
    assert!(html.contains("需要确认才能继续"));
    assert!(html.contains("id=\"run-state\""));
}

#[test]
fn rust_shell_does_not_take_key_as_flag() {
    let main = include_str!("../src/main.rs");
    let sidecar = include_str!("../src/sidecar.rs");
    let gui = include_str!("../src/gui.rs");
    assert!(!main.contains("--api-key"));
    assert!(!main.contains("--api_key"));
    assert!(sidecar.contains("Never pass the key as an argument"));
    assert!(!main.contains("updater"));
    assert!(!gui.contains("updater"));
    assert!(!gui.contains("Updater"));
}
