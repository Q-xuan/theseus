//! Desktop chrome is a client projection. It must not grow a loop or a key box.

#[test]
fn desktop_ui_stays_a_client() {
    let html = include_str!("../ui/index.html");
    let js = include_str!("../ui/app.js");
    let css = include_str!("../ui/app.css");
    for hay in [html, js, css] {
        assert!(!hay.contains("THESEUS_LLM_API_KEY"));
        assert!(!hay.contains("PI_LLM_API_KEY"));
        assert!(!hay.contains("apiKey"));
        assert!(!hay.contains("api_key"));
        assert!(!hay.contains("password"));
        assert!(!hay.contains("derive_messages"));
        assert!(!hay.contains("MCP"));
        assert!(!hay.contains("compaction"));
        assert!(!hay.contains("THESEUS_SESSIONS_DIR"));
        assert!(!hay.contains("PI_SESSIONS_DIR"));
        assert!(!hay.contains("THESEUS_HOME"));
        assert!(!hay.contains("PI_HOME"));
        assert!(!hay.contains("THESEUS_TOOL_APPROVAL"));
        assert!(!hay.contains("PI_TOOL_APPROVAL"));
        assert!(!hay.contains("\"thread/start\""));
        assert!(!hay.contains("\"thread/resume\""));
        assert!(!hay.contains("\"thread/list\""));
        assert!(!hay.contains("\"turn/start\""));
        assert!(!hay.contains("\"turn/interrupt\""));
        assert!(!hay.contains("\"tool/approve\""));
        assert!(!hay.contains("\"tool/reject\""));
        assert!(!hay.contains("theseus-app-server"));
    }
    assert!(html.contains("data-app=\"theseus-desktop\""));
    assert!(html.contains("id=\"sessions\""), "sidebar session list");
    assert!(html.contains("最近"), "recency list label");
    assert!(html.contains("id=\"dock\""));
    assert!(html.contains("新对话"));
    assert!(html.contains("id=\"copy-thread-id\""), "quiet optional copy affordance");
    assert!(!html.contains("复制 id"), "session id is not a primary path");
    assert!(!html.contains("id=\"thread-id-box\""));
    assert!(!html.contains("id=\"meta\""), "one workspace chip only");
    assert!(html.contains("id=\"prefs\""), "one closable settings card");
    assert!(html.contains("id=\"open-prefs\""), "gear / settings entry");
    assert!(html.contains("id=\"stop\""), "composer Stop");
    assert!(html.contains("id=\"workspace-chip\""), "current workspace");
    assert!(html.contains("id=\"empty-cta\""), "centered empty CTA");
    assert!(!html.contains("id=\"approval\""), "Theseus approval gate is off the product path");
    assert!(!html.contains("id=\"prefs-key\""), "keys stay inside pi");
    assert!(!html.contains("危险工具"), "defer gate tip off the empty state");
    assert!(!html.contains(">停止<"), "Stop is a circular icon, not a text button");
    assert!(!html.contains("type=\"search\""));
    assert!(!html.contains("short-link"));
    assert!(!html.contains("oauth"));
    assert!(!html.contains("OAuth"));
    assert!(html.contains("badlogic/pi-mono"), "settings cite pi");
    assert!(!js.contains("shareUrl"));
    assert!(!js.contains("shortLink"));
    assert!(
        !js.contains("THESEUS_LLM_MODEL"),
        "model strip talks to /settings; env names stay in the shell"
    );
    assert!(html.contains("id=\"model-strip\""), "composer model strip");
    assert!(html.contains("id=\"timeline\""), "left turn timeline");
    assert!(js.contains("/settings"), "thin settings endpoint");
    assert!(js.contains("turn-anchor"), "clickable turn marks");
    assert!(js.contains("scrollToTurn"));
    assert!(js.contains("dataset.turn"), "turn marks carry a turn index");
    assert!(js.contains("item/agentMessage/delta"));
    assert!(js.contains("new_session"));
    assert!(js.contains("switch_session"));
    assert!(js.contains("list_sessions"));
    assert!(js.contains("prompt"));
    assert!(js.contains("abort"), "Stop uses pi abort");
    assert!(js.contains("clear_queue"));
    assert!(js.contains("get_state"));
    assert!(js.contains("get_messages") || js.contains("get_entries"));
    assert!(js.contains("set_session_name"));
    assert!(js.contains("agent_settled"));
    assert!(js.contains("已停止"), "Stop copy aligns with settled");
    assert!(js.contains("isStreaming"));
    assert!(js.contains("requestAnimationFrame") || js.contains("BATCH_MS"));
    assert!(js.contains("readableError"));
    assert!(js.contains("turn-mark"));
    assert!(js.contains("tool-head"));
    assert!(js.contains("clipboard.writeText") || js.contains("copyText"));
    assert!(js.contains("shortLabel"), "titles and sidebar do not dump long prompts");
    assert!(js.contains("TITLE_MAX = 28"), "header title is first-sentence + hard cap");
    assert!(js.contains("firstSentence"));
    assert!(js.contains("[。！？!?]"), "title cuts at the first sentence");
    assert!(js.contains("relativeTime"), "sidebar shows relative time");
    assert!(
        js.contains("setTitle(text, \"新对话\")"),
        "after send / while streaming, title truncates the user prompt"
    );
    assert!(!js.contains("titleEl.textContent = text"));
    assert!(!js.contains("class=\"who\""), "no 你/助手 role tags");
    assert!(
        !js.contains("session/event"),
        "UI must project Item/delta, not raw session log"
    );
    assert!(
        !html.contains("data-app=\"theseus-web\""),
        "desktop chrome is not the theseus-web skin"
    );
}

#[test]
fn chrome_cites_dsh_modules_not_a_pixel_clone() {
    let css = include_str!("../ui/app.css");
    assert!(css.contains("SidebarRoot.module.css"));
    assert!(css.contains("GenericToolCard"));
    assert!(css.contains("28px"), "compact new-session bar height");
    assert!(css.contains("#0c0c0c"), "Codex-like near-black paper");
    assert!(css.contains("#161616"), "charcoal rail");
    assert!(css.contains("#1a2a40"), "muted-blue selected row");
    let html = include_str!("../ui/index.html");
    assert!(!html.contains("id=\"run-state\""), "busy lives on spinner + Stop");
    assert!(css.contains("composer-strip"), "model strip on composer bottom");
    assert!(css.contains(".timeline"), "turn ticks");
    assert!(css.contains(".stop-sq"), "circular Stop with square inside");
    assert!(css.contains("border-radius: 50%"), "circular Stop");
    assert!(css.contains(".session-time"), "sidebar relative time");
    assert!(!html.contains("Plugins"));
    assert!(!html.contains("Pull requests"));
    assert!(!html.contains("Scheduled"));
    assert!(!html.contains("Explore"));
    assert!(!html.contains("Pinned"));
    assert!(html.contains("id=\"prefs\""), "one card, not a settings maze");
    assert!(!html.contains("OAuth"));
    assert!(css.contains(".prefs"), "thin prefs card");
    assert!(css.contains("prefs-open"), "settings card does not fight empty state");
}

#[test]
fn rust_shell_does_not_take_key_as_flag() {
    let main = include_str!("../src/main.rs");
    let sidecar = include_str!("../src/sidecar.rs");
    let gui = include_str!("../src/gui.rs");
    assert!(!main.contains("--api-key"));
    assert!(!main.contains("--api_key"));
    assert!(sidecar.contains("sidecar argv must not carry secrets"));
    assert!(sidecar.contains("--mode"));
    assert!(sidecar.contains("rpc"));
    assert!(!main.contains("updater"));
    assert!(!gui.contains("updater"));
    assert!(!gui.contains("Updater"));
}

#[test]
fn release_gui_uses_windows_subsystem_and_quiet_stdio() {
    let main = include_str!("../src/main.rs");
    assert!(
        main.contains("windows_subsystem"),
        "Release + gui must not open a Windows console"
    );
    assert!(main.contains("feature = \"gui\""));
    assert!(main.contains("not(debug_assertions)"));
    let gui = include_str!("../src/gui.rs");
    assert!(
        gui.contains("verbose_stdio"),
        "product GUI must gate UI-on-http / sessions spam"
    );
    assert!(gui.contains("theseus-desktop UI on"));
    let sidecar = include_str!("../src/sidecar.rs");
    assert!(sidecar.contains("CREATE_NO_WINDOW"));
    assert!(sidecar.contains("verbose_stdio"));
    let lib = include_str!("../src/lib.rs");
    assert!(lib.contains("fn verbose_stdio"));
}

#[test]
fn product_path_is_pi_rpc_only() {
    let sidecar = include_str!("../src/sidecar.rs");
    let bridge = include_str!("../src/bridge.rs");
    let map = include_str!("../src/map.rs");
    assert!(sidecar.contains("pi --mode rpc") || sidecar.contains("\"rpc\""));
    assert!(map.contains("is_theseus_app_server_method"));
    assert!(bridge.contains("product path is pi --mode rpc only"));
    assert!(bridge.contains("pi-rpc"));
    assert!(!sidecar.contains("theseus-app-server"));
}
