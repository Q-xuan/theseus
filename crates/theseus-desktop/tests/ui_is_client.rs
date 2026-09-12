//! Desktop chrome is a client projection. It must not grow a loop or a key box.

#[test]
fn desktop_ui_stays_a_client() {
    let html = include_str!("../ui/index.html");
    let js = include_str!("../ui/app.js");
    let css = include_str!("../ui/app.css");
    assert!(
        js.contains("未配置 THESEUS_LLM_API_KEY"),
        "missing-key banner is a short env-var hint"
    );
    let js_without_banner = js.replace("未配置 THESEUS_LLM_API_KEY", "");
    let html_without_key_field = html.replace(
        r#"<input id="prefs-key" type="password" autocomplete="off" spellcheck="false" placeholder="粘贴后只写入本机用户环境 / 钥匙串" />"#,
        "",
    );
    for hay in [html_without_key_field.as_str(), js_without_banner.as_str(), css] {
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
        assert!(!hay.contains(".theseus"));
        assert!(!hay.contains(".pi-app"));
        assert!(!hay.contains("THESEUS_TOOL_APPROVAL"));
        assert!(!hay.contains("PI_TOOL_APPROVAL"));
    }
    assert!(html.contains("data-app=\"theseus-desktop\""));
    assert!(html.contains("id=\"sessions\""), "sidebar session list");
    assert!(html.contains("最近"), "recency list label");
    assert!(html.contains("id=\"approval\""));
    assert!(
        html.contains("id=\"dock\""),
        "composer dock for the one gate"
    );
    assert!(html.contains("新对话"));
    assert!(html.contains("id=\"copy-thread-id\""), "quiet optional copy affordance");
    assert!(!html.contains("复制 id"), "thread id is not a primary path");
    assert!(!html.contains("id=\"thread-id-box\""));
    assert!(!html.contains("id=\"meta\""), "one workspace chip only");
    assert!(html.contains("id=\"prefs\""), "one closable settings card");
    assert!(html.contains("id=\"open-prefs\""), "gear / settings entry");
    assert!(html.contains("id=\"stop\""), "composer Stop");
    assert!(html.contains("id=\"workspace-chip\""), "current workspace");
    assert!(html.contains("id=\"empty-cta\""), "centered empty CTA");
    assert!(html.contains("id=\"composer-input\""), "gate replaces composer input");
    assert!(html.contains("id=\"stopped\""), "已停止 sits on the Stop beat");
    assert!(html.contains("已停止"), "stopped label copy");
    assert!(!html.contains("危险工具"), "defer gate tip off the empty state");
    assert!(!html.contains(">停止<"), "Stop is a circular icon, not a text button");
    assert!(!html.contains("type=\"search\""));
    assert!(!html.contains("short-link"));
    assert!(!html.contains("oauth"));
    assert!(!html.contains("OAuth"));
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
    assert!(js.contains("thread/start"));
    assert!(js.contains("thread/resume"));
    assert!(js.contains("thread/list"));
    assert!(js.contains("turn/start"));
    assert!(js.contains("turn/interrupt"), "Stop uses existing end semantics");
    assert!(js.contains("tool/approve"));
    assert!(js.contains("tool/reject"));
    assert!(js.contains("item/tool/approval/request"));
    assert!(js.contains("readableError"));
    assert!(js.contains("turn-mark"));
    assert!(js.contains("tool-head"));
    assert!(js.contains("clipboard.writeText") || js.contains("copyText"));
    assert!(js.contains("showThreadId"));
    assert!(js.contains("shortLabel"), "titles and sidebar do not dump long prompts");
    assert!(js.contains("TITLE_MAX = 28"), "header title is first-sentence + hard cap");
    assert!(js.contains("firstSentence"));
    assert!(js.contains("[。！？!?]"), "title cuts at the first sentence");
    assert!(
        js.contains("setTitle(text, \"新对话\")"),
        "after send / while streaming, title truncates the user prompt"
    );
    assert!(!js.contains("titleEl.textContent = text"));
    assert!(!js.contains("class=\"who\""), "no 你/助手 role tags");
    assert!(!js.contains("t.preview || t.id"), "sidebar does not fall back to thr_*");
    assert!(js.contains("busyThreadId"), "amber dot tracks the in-progress thread");
    assert!(js.contains("relativeTime"), "sidebar shows relative updatedAt");
    assert!(js.contains("t.updatedAt"), "sidebar row uses list updatedAt");
    assert!(js.contains("session-title"), "sidebar row is title + time");
    assert!(js.contains("FOLLOW_THRESHOLD"), "unfollow after scrolling up");
    assert!(js.contains("followIfPinned"), "one conversation follow-scroll");
    assert!(js.contains("setBusy(false, stopped)"), "Stop and 已停止 share one busy beat");
    assert!(js.contains("composer-input"), "gate takes over the composer input");
    assert!(js.contains("classList.add(\"gated\")"), "approval card replaces the input");
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
    assert!(css.contains("ApprovalPanel"));
    assert!(css.contains("GenericToolCard"));
    assert!(css.contains("28px"), "compact new-session bar height");
    assert!(css.contains("gate-strip"), "amber approval strip");
    assert!(css.contains("#0c0c0c"), "Codex-like near-black paper");
    assert!(css.contains("#161616"), "charcoal rail");
    assert!(css.contains("#1a2a40"), "muted-blue selected row");
    let html = include_str!("../ui/index.html");
    assert!(html.contains("需要确认才能继续"));
    assert!(!html.contains("id=\"run-state\""), "busy lives on spinner + Stop");
    assert!(css.contains("composer-strip"), "model strip on composer bottom");
    assert!(css.contains(".timeline"), "turn ticks");
    assert!(css.contains("height: 16px"), "timeline ticks are a visible hit target");
    assert!(css.contains("height: 24px"), "tool row title + summary is 24px");
    assert!(css.contains(".session-time"), "sidebar relative time");
    assert!(css.contains("composer.gated"), "gate occupies the composer slot");
    assert!(css.contains("position: sticky"), "composer stays put under the one scroll");
    assert!(css.contains(".stop-sq"), "circular Stop with square inside");
    assert!(css.contains("border-radius: 50%"), "circular Stop");
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
    assert!(sidecar.contains("Never pass the key as an argument"));
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
