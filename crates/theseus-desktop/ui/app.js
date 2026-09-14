/* Thin client. Projects Thread / Turn / Item. Sends pi RPC intents. No loop. */
(() => {
  const $ = (id) => document.getElementById(id);
  const logEl = $("log");
  const emptyEl = $("empty");
  const emptyCtaEl = $("empty-cta");
  const bannerEl = $("banner");
  const titleEl = $("title");
  const copyThreadIdEl = $("copy-thread-id");
  const inputEl = $("input");
  const sendEl = $("send");
  const newEl = $("new-thread");
  const sessionsEl = $("sessions");
  const sessionsEmptyEl = $("sessions-empty");
  const formEl = $("composer");
  const railEl = $("rail");
  const toggleRailEl = $("toggle-rail");
  const timelineEl = $("timeline");
  const threadEl = $("thread");
  const modelStripEl = $("model-strip");
  const modelNameEl = $("model-name");
  const modelVariantEl = $("model-variant");
  const modelBusyEl = $("model-busy");
  const stopEl = $("stop");
  const openPrefsEl = $("open-prefs");
  const closePrefsEl = $("close-prefs");
  const prefsMaskEl = $("prefs-mask");
  const prefsWorkspaceEl = $("prefs-workspace");
  const prefsPickEl = $("prefs-pick-workspace");
  const prefsSaveEl = $("prefs-save");
  const workspaceChipEl = $("workspace-chip");

  let ws = null;
  let nextId = 1;
  const pending = new Map();
  let sessionId = null;
  let sessionPath = null;
  let sessionPreview = "";
  let busy = false;
  let wantsStop = false;
  const items = new Map();
  let recent = [];
  let turnCount = 0;
  let currentModel = "";
  let currentWorkspace = "";
  let activeTurn = 0;
  let pendingDeltas = new Map();
  let flushTimer = 0;
  const BATCH_MS = 24;

  function readableError(err) {
    const raw = (err && err.message) || (typeof err === "string" ? err : "");
    if (/not found on PATH|pi --mode rpc/i.test(raw)) {
      return "未找到 pi。请安装 pi 并加入 PATH：https://github.com/badlogic/pi-mono";
    }
    if (/API_KEY|provider|auth|unauthor/i.test(raw)) {
      return "pi 未配置密钥或模型。请在 pi 里完成 provider 设置。";
    }
    return raw || "请求失败。";
  }

  const TITLE_MAX = 28;

  function firstSentence(text) {
    const raw = String(text || "").replace(/\s+/g, " ").trim();
    if (!raw) return "";
    const stop = raw.search(/[。！？!?]/);
    if (stop >= 0) return raw.slice(0, stop + 1).trim();
    return raw.split(/[\n\r]/)[0].trim() || raw;
  }

  function shortLabel(text, fallback) {
    const sentence = firstSentence(text);
    if (!sentence || /^sess[_A-Za-z0-9-]+$/.test(sentence) || /^thr_[A-Za-z0-9_-]+$/.test(sentence)) {
      return fallback || "新对话";
    }
    const chars = Array.from(sentence);
    if (chars.length <= TITLE_MAX) return sentence;
    return `${chars.slice(0, TITLE_MAX).join("")}…`;
  }

  function userItemText(item) {
    const content = item && item.content;
    if (typeof content === "string") return content;
    if (Array.isArray(content)) {
      return content.map((c) => (c && c.text) || "").join("\n");
    }
    return "";
  }

  function workspaceLabel(path) {
    const raw = String(path || "").trim();
    if (!raw) return "";
    const parts = raw.replace(/\\/g, "/").split("/").filter(Boolean);
    return parts[parts.length - 1] || raw;
  }

  function setTitle(raw, fallback) {
    titleEl.textContent = shortLabel(raw, fallback || (sessionId ? "新对话" : "未打开对话"));
  }

  function showSession(id, path) {
    sessionId = id || null;
    sessionPath = path || sessionPath || null;
    copyThreadIdEl.classList.toggle("hidden", !sessionId);
    copyThreadIdEl.disabled = !sessionId;
    copyThreadIdEl.setAttribute("aria-label", "复制 session id");
    titleEl.title = sessionId || "";
    syncEmpty();
  }

  function copyText(text) {
    const fallback = () =>
      new Promise((resolve, reject) => {
        const ta = document.createElement("textarea");
        ta.value = text;
        ta.setAttribute("readonly", "");
        ta.style.position = "fixed";
        ta.style.left = "-9999px";
        document.body.appendChild(ta);
        ta.select();
        try {
          if (!document.execCommand("copy")) reject(new Error("copy"));
          else resolve();
        } catch (err) {
          reject(err);
        } finally {
          document.body.removeChild(ta);
        }
      });
    if (navigator.clipboard && navigator.clipboard.writeText) {
      return Promise.race([
        navigator.clipboard.writeText(text),
        new Promise((_, reject) => {
          setTimeout(() => reject(new Error("clipboard-timeout")), 400);
        }),
      ]).catch(() => fallback());
    }
    return fallback();
  }

  function showBanner(text, kind) {
    if (!text) {
      bannerEl.classList.add("hidden");
      bannerEl.classList.remove("warn");
      bannerEl.textContent = "";
      return;
    }
    bannerEl.textContent = text;
    bannerEl.classList.toggle("warn", kind === "warn");
    bannerEl.classList.remove("hidden");
  }

  function setBusy(on) {
    busy = on;
    modelBusyEl.classList.toggle("on", on);
    sendEl.classList.toggle("hidden", on);
    stopEl.classList.toggle("hidden", !on);
    const ready = Boolean(ws && ws.readyState === 1 && sessionId && !busy);
    inputEl.disabled = !ready;
    sendEl.disabled = !ready;
    stopEl.disabled = !on;
    const connected = Boolean(ws && ws.readyState === 1 && !busy);
    newEl.disabled = !connected;
    emptyCtaEl.disabled = !connected;
  }

  function clearBusyChrome() {
    busy = false;
    wantsStop = false;
    modelBusyEl.classList.remove("on");
    stopEl.classList.add("hidden");
    stopEl.disabled = true;
    sendEl.classList.remove("hidden");
  }

  function renderWorkspace(path) {
    currentWorkspace = path || currentWorkspace || "";
    const label = workspaceLabel(currentWorkspace);
    workspaceChipEl.textContent = label;
    workspaceChipEl.title = currentWorkspace;
    workspaceChipEl.classList.toggle("hidden", !label);
    if (prefsWorkspaceEl && document.activeElement !== prefsWorkspaceEl) {
      prefsWorkspaceEl.value = currentWorkspace;
    }
  }

  function splitModelLabel(name) {
    const parts = String(name || "").trim().split(/\s+/).filter(Boolean);
    if (parts.length >= 2) {
      return { name: parts.slice(0, -1).join(" "), variant: parts[parts.length - 1] };
    }
    return { name: parts[0] || "", variant: "" };
  }

  function renderModel() {
    const { name, variant } = splitModelLabel(currentModel);
    modelNameEl.textContent = name || "pi";
    modelVariantEl.textContent = variant;
  }

  function applySettingsPayload(data) {
    if (data && data.model) currentModel = data.model;
    if (data && data.workspace) renderWorkspace(data.workspace);
    renderModel();
  }

  async function loadModel() {
    try {
      const res = await fetch("/settings");
      const data = await res.json();
      applySettingsPayload(data);
    } catch {
      currentModel = currentModel || "";
      renderModel();
    }
  }

  function openPrefs() {
    document.body.classList.add("prefs-open");
    prefsMaskEl.classList.remove("hidden");
    prefsWorkspaceEl.value = currentWorkspace;
    prefsWorkspaceEl.focus();
  }

  function closePrefs() {
    document.body.classList.remove("prefs-open");
    prefsMaskEl.classList.add("hidden");
  }

  async function savePrefs() {
    const payload = { workspace: prefsWorkspaceEl.value.trim() };
    try {
      const res = await fetch("/settings", {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(payload),
      });
      const data = await res.json();
      applySettingsPayload(data);
      closePrefs();
    } catch {
      showBanner("设置没存上。");
    }
  }

  function setActiveTurn(n) {
    activeTurn = n;
    timelineEl.querySelectorAll(".tick").forEach((tick) => {
      tick.classList.toggle("active", Number(tick.dataset.turn) === n);
    });
  }

  function scrollToTurn(n) {
    const mark = document.getElementById(`turn-${n}`);
    if (!mark) return;
    mark.scrollIntoView({ block: "start", behavior: "smooth" });
    setActiveTurn(n);
  }

  function renderTimeline() {
    timelineEl.innerHTML = "";
    for (let n = 1; n <= turnCount; n += 1) {
      const tick = document.createElement("button");
      tick.type = "button";
      tick.className = "tick" + (n === activeTurn ? " active" : "");
      tick.dataset.turn = String(n);
      tick.setAttribute("aria-label", `回合 ${n}`);
      tick.addEventListener("click", () => scrollToTurn(n));
      timelineEl.appendChild(tick);
    }
    if (!activeTurn && turnCount > 0) setActiveTurn(1);
  }

  function syncActiveTurn() {
    const marks = logEl.querySelectorAll(".turn-anchor");
    if (!marks.length) return;
    const top = threadEl.getBoundingClientRect().top + 16;
    let current = marks[0];
    marks.forEach((mark) => {
      if (mark.getBoundingClientRect().top <= top + 48) current = mark;
    });
    const n = Number(current.dataset.turn);
    if (n) setActiveTurn(n);
  }

  function pretty(raw) {
    try {
      return JSON.stringify(JSON.parse(raw), null, 2);
    } catch {
      return raw || "";
    }
  }

  function briefArgs(raw) {
    if (raw == null) return "";
    if (typeof raw === "object") {
      return raw.path || raw.command || raw.oldString || JSON.stringify(raw);
    }
    try {
      const parsed = JSON.parse(raw);
      return parsed.path || parsed.command || parsed.oldString || pretty(raw);
    } catch {
      return String(raw);
    }
  }

  function relativeTime(ms) {
    const t = Number(ms);
    if (!t) return "";
    const diff = Date.now() - t;
    if (diff < 45 * 1000) return "刚刚";
    if (diff < 60 * 60 * 1000) return `${Math.max(1, Math.round(diff / 60000))} 分钟前`;
    if (diff < 24 * 60 * 60 * 1000) return `${Math.max(1, Math.round(diff / 3600000))} 小时前`;
    if (diff < 7 * 24 * 60 * 60 * 1000) return `${Math.max(1, Math.round(diff / 86400000))} 天前`;
    const d = new Date(t);
    return `${d.getMonth() + 1}/${d.getDate()}`;
  }

  function syncEmpty() {
    emptyEl.classList.toggle("hidden", Boolean(sessionId) || logEl.children.length > 0);
  }

  function upsert(item) {
    items.set(item.id, item);
    let li = document.getElementById(`item-${item.id}`);
    if (!li) {
      li = document.createElement("li");
      li.id = `item-${item.id}`;
      li.className = "item";
      logEl.appendChild(li);
    }
    const type = item.type;
    li.classList.remove("user", "agent", "tool", "error");
    if (type === "userMessage") {
      li.classList.add("user");
      const text = (item.content || []).map((c) => c.text || "").join("\n");
      li.innerHTML = `<div class="bubble">${escapeHtml(text)}</div>`;
    } else if (type === "agentMessage") {
      li.classList.add("agent");
      li.innerHTML = `<div class="bubble">${escapeHtml(item.text || "")}</div>`;
    } else if (type === "toolCall" || type === "toolResult") {
      li.classList.add("tool");
      const isErr = type === "toolResult" && item.isError;
      if (isErr) li.classList.add("error");
      const name = type === "toolCall" ? item.name || "tool" : isErr ? "失败" : "结果";
      const brief =
        type === "toolCall" ? briefArgs(item.arguments) : String(item.output || "").split("\n")[0];
      const body =
        type === "toolCall" ? pretty(item.arguments || "") : String(item.output || "");
      const open = type === "toolResult" && isErr;
      li.innerHTML = `<div class="tool-card"><button type="button" class="tool-head"><span class="tool-name">${escapeHtml(
        name
      )}</span><span class="tool-brief">${escapeHtml(brief)}</span></button><pre class="tool-body${
        open ? "" : " hidden"
      }">${escapeHtml(body)}</pre></div>`;
      li.querySelector(".tool-head").addEventListener("click", () => {
        li.querySelector(".tool-body").classList.toggle("hidden");
      });
    } else {
      li.classList.add("tool");
      li.textContent = type || "item";
    }
    syncEmpty();
    li.scrollIntoView({ block: "end" });
  }

  function queueAgentDelta(id, delta) {
    const prev = items.get(id) || { type: "agentMessage", id, text: "" };
    prev.text = (prev.text || "") + delta;
    items.set(id, prev);
    pendingDeltas.set(id, prev);
    if (flushTimer) return;
    const run = () => {
      flushTimer = 0;
      pendingDeltas.forEach((item) => upsert(item));
      pendingDeltas.clear();
    };
    if (typeof requestAnimationFrame === "function") {
      flushTimer = requestAnimationFrame(() => {
        flushTimer = setTimeout(run, 0);
      });
    } else {
      flushTimer = setTimeout(run, BATCH_MS);
    }
  }

  function addTurnMark() {
    const n = turnCount;
    const li = document.createElement("li");
    li.className = "turn-mark";
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "turn-anchor";
    btn.id = `turn-${n}`;
    btn.dataset.turn = String(n);
    btn.setAttribute("aria-label", `回合 ${n}`);
    btn.addEventListener("click", () => scrollToTurn(n));
    li.appendChild(btn);
    logEl.appendChild(li);
    syncEmpty();
    renderTimeline();
    setActiveTurn(n);
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  function clearLog() {
    items.clear();
    pendingDeltas.clear();
    logEl.innerHTML = "";
    turnCount = 0;
    activeTurn = 0;
    timelineEl.innerHTML = "";
    syncEmpty();
  }

  function renderSessions() {
    sessionsEl.querySelectorAll(".session").forEach((n) => n.remove());
    sessionsEmptyEl.classList.toggle("hidden", recent.length > 0);
    for (const t of recent) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "session" + (t.id === sessionId || t.path === sessionPath ? " active" : "");
      btn.dataset.id = t.id;
      const title = shortLabel(t.title || t.preview, "新对话");
      const time = relativeTime(t.updatedAt);
      btn.innerHTML = `<span class="session-dot${
        busy && (t.id === sessionId || t.path === sessionPath) ? "" : " off"
      }"></span><span class="session-main"><span class="session-preview">${escapeHtml(
        title
      )}</span><span class="session-time">${escapeHtml(time)}</span></span>`;
      btn.addEventListener("click", () => {
        railEl.classList.remove("open");
        resumeSession(t).catch((err) => {
          showBanner(readableError(err) || "打不开这个会话。");
        });
      });
      sessionsEl.appendChild(btn);
    }
  }

  function applyThread(thread, extra) {
    if (!thread) return;
    showSession(thread.id || (extra && extra.sessionId), thread.path || (extra && extra.sessionFile));
    sessionPreview = shortLabel(thread.preview || thread.title, "新对话");
    setTitle(thread.preview || thread.title, "新对话");
    if (thread.cwd) renderWorkspace(thread.cwd);
    clearLog();
    for (const turn of thread.turns || []) {
      turnCount += 1;
      addTurnMark();
      for (const item of turn.items || []) {
        if (item && item.id) upsert(item);
      }
    }
    setBusy(false);
    inputEl.focus();
    renderSessions();
  }

  function applyState(state) {
    if (!state) return;
    if (state.sessionId) showSession(state.sessionId, state.sessionFile);
    if (state.sessionFile) sessionPath = state.sessionFile;
    if (state.sessionName) {
      sessionPreview = shortLabel(state.sessionName, "新对话");
      setTitle(state.sessionName, "新对话");
    }
    if (state.model) {
      currentModel = state.model.id || state.model.name || currentModel;
      renderModel();
    }
    if (state.isStreaming === true) setBusy(true);
    if (state.isStreaming === false) setBusy(false);
  }

  function onNotify(msg) {
    switch (msg.method) {
      case "agent_start": {
        setBusy(true);
        break;
      }
      case "agent_settled": {
        const streaming = msg.params && msg.params.isStreaming;
        if (streaming === false || streaming == null) {
          if (wantsStop) showBanner("已停止", "warn");
          wantsStop = false;
          setBusy(false);
          loadRecent();
        }
        break;
      }
      case "thread/started": {
        const thread = msg.params && msg.params.thread;
        if (!thread) return;
        showSession(thread.id, thread.path);
        sessionPreview = shortLabel(thread.preview, "新对话");
        setTitle(thread.preview, "新对话");
        break;
      }
      case "turn/started": {
        turnCount += 1;
        addTurnMark();
        break;
      }
      case "item/started":
      case "item/completed": {
        const item = msg.params && msg.params.item;
        if (item && item.id) upsert(item);
        if (item && item.type === "userMessage") {
          const text = userItemText(item);
          if (text) {
            sessionPreview = shortLabel(text, "新对话");
            setTitle(text, "新对话");
          }
        }
        break;
      }
      case "item/agentMessage/delta": {
        const id = msg.params && msg.params.itemId;
        const delta = (msg.params && msg.params.delta) || "";
        if (!id || !delta) return;
        queueAgentDelta(id, delta);
        break;
      }
      case "turn/completed": {
        const turn = msg.params && msg.params.turn;
        if (turn && turn.status === "failed") {
          showBanner("这一轮失败了。");
        }
        break;
      }
      default:
        break;
    }
  }

  function rpc(method, params) {
    const id = nextId++;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params: params || {} }));
    });
  }

  async function loadRecent() {
    if (!ws || ws.readyState !== 1) return;
    let result;
    try {
      result = await rpc("list_sessions", {});
    } catch (err) {
      showBanner(readableError(err));
      return;
    }
    recent = (result && result.sessions) || [];
    renderSessions();
  }

  async function refreshState() {
    const state = await rpc("get_state", {});
    applyState(state);
    return state;
  }

  async function startSession() {
    showBanner("");
    clearLog();
    showSession(null);
    setTitle("新对话", "新对话");
    await rpc("new_session", {});
    const state = await refreshState();
    showSession(state.sessionId, state.sessionFile);
    setBusy(false);
    inputEl.focus();
    await loadRecent();
  }

  async function resumeSession(row) {
    if (!row || !row.path) return;
    showBanner("");
    await rpc("switch_session", { sessionPath: row.path });
    let projected;
    try {
      projected = await rpc("get_messages", {});
    } catch {
      projected = await rpc("get_entries", {});
    }
    const state = await refreshState().catch(() => ({}));
    applyThread(projected && projected.thread, {
      sessionId: row.id || (state && state.sessionId),
      sessionFile: row.path,
    });
  }

  async function sendPrompt(text) {
    if (!sessionId || busy) return;
    showBanner("");
    setTitle(text, "新对话");
    sessionPreview = shortLabel(text, "新对话");
    wantsStop = false;
    setBusy(true);
    try {
      await rpc("prompt", { message: text });
      const name = shortLabel(text, "");
      if (name && name !== "新对话") {
        rpc("set_session_name", { name }).catch(() => {});
      }
    } catch (err) {
      showBanner(readableError(err), "warn");
      setBusy(false);
    }
  }

  async function stopTurn() {
    if (!busy) return;
    wantsStop = true;
    try {
      await rpc("clear_queue", {}).catch(() => {});
      await rpc("abort", {});
    } catch (err) {
      showBanner(readableError(err));
      setBusy(false);
    }
  }

  function connect() {
    setTitle("未打开对话", "未打开对话");
    showSession(null);
    const proto = location.protocol === "https:" ? "wss" : "ws";
    ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.onopen = () => {
      setBusy(false);
      refreshState()
        .then((state) => {
          if (state && state.messageCount > 0) {
            return rpc("get_messages", {}).then((projected) => {
              applyThread(projected && projected.thread, state);
            });
          }
          if (state && state.sessionId) showSession(state.sessionId, state.sessionFile);
          return loadRecent();
        })
        .catch(() => {});
    };
    ws.onmessage = (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      if (msg.method) {
        onNotify(msg);
        return;
      }
      if (msg.id == null) return;
      const wait = pending.get(msg.id);
      if (!wait) return;
      pending.delete(msg.id);
      if (msg.error) wait.reject(msg.error);
      else wait.resolve(msg.result);
    };
    ws.onclose = () => {
      clearBusyChrome();
      inputEl.disabled = true;
      sendEl.disabled = true;
      newEl.disabled = true;
      emptyCtaEl.disabled = true;
      showBanner("和 sidecar 的连接断了。关掉窗口再开（Linux 预览则刷新页面）。");
    };
  }

  formEl.addEventListener("submit", (e) => {
    e.preventDefault();
    const text = inputEl.value.trim();
    if (!text) return;
    inputEl.value = "";
    sendPrompt(text);
  });

  inputEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      formEl.requestSubmit();
    }
  });

  stopEl.addEventListener("click", () => {
    stopTurn();
  });

  function startFromCta() {
    startSession().catch((err) => {
      showBanner(readableError(err) || "开不了新对话。");
    });
  }

  newEl.addEventListener("click", startFromCta);
  emptyCtaEl.addEventListener("click", startFromCta);

  openPrefsEl.addEventListener("click", openPrefs);
  closePrefsEl.addEventListener("click", closePrefs);
  prefsMaskEl.addEventListener("click", (e) => {
    if (e.target === prefsMaskEl) closePrefs();
  });
  prefsSaveEl.addEventListener("click", () => {
    savePrefs();
  });
  prefsPickEl.addEventListener("click", () => {
    fetch("/settings/workspace-pick", { method: "POST" })
      .then((res) => res.json())
      .then((data) => applySettingsPayload(data))
      .catch(() => {});
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !prefsMaskEl.classList.contains("hidden")) {
      closePrefs();
    }
  });

  toggleRailEl.addEventListener("click", () => {
    railEl.classList.toggle("open");
  });

  threadEl.addEventListener("scroll", syncActiveTurn, { passive: true });

  copyThreadIdEl.addEventListener("click", () => {
    if (!sessionId) return;
    copyText(sessionId)
      .then(() => {
        copyThreadIdEl.setAttribute("aria-label", "已复制");
        setTimeout(() => {
          if (copyThreadIdEl.getAttribute("aria-label") === "已复制") {
            copyThreadIdEl.setAttribute("aria-label", "复制 session id");
          }
        }, 1200);
      })
      .catch(() => {
        copyThreadIdEl.setAttribute("aria-label", "复制失败");
        setTimeout(() => {
          if (copyThreadIdEl.getAttribute("aria-label") === "复制失败") {
            copyThreadIdEl.setAttribute("aria-label", "复制 session id");
          }
        }, 1200);
      });
  });

  loadModel();
  connect();
})();
