/* Thin JSON-RPC client. Projects Thread / Turn / Item + one gate. No loop. */
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
  const composerInputEl = $("composer-input");
  const approvalEl = $("approval");
  const approvalToolEl = $("approval-tool");
  const approvalSummaryEl = $("approval-summary");
  const approvalApproveEl = $("approval-approve");
  const approvalRejectEl = $("approval-reject");
  const stoppedEl = $("stopped");
  const railEl = $("rail");
  const toggleRailEl = $("toggle-rail");
  const timelineEl = $("timeline");
  const threadEl = $("thread");
  const modelStripEl = $("model-strip");
  const modelNameEl = $("model-name");
  const modelVariantEl = $("model-variant");
  const modelPopEl = $("model-pop");
  const modelInputEl = $("model-input");
  const modelBusyEl = $("model-busy");
  const stopEl = $("stop");
  const openPrefsEl = $("open-prefs");
  const closePrefsEl = $("close-prefs");
  const prefsMaskEl = $("prefs-mask");
  const prefsBaseEl = $("prefs-base-url");
  const prefsModelEl = $("prefs-model");
  const prefsWorkspaceEl = $("prefs-workspace");
  const prefsPickEl = $("prefs-pick-workspace");
  const prefsKeyEl = $("prefs-key");
  const prefsKeyStatusEl = $("prefs-key-status");
  const prefsSaveEl = $("prefs-save");
  const workspaceChipEl = $("workspace-chip");

  let ws = null;
  let nextId = 1;
  const pending = new Map();
  let threadId = null;
  let threadPreview = "";
  let busy = false;
  let busyThreadId = null;
  const items = new Map();
  let approval = null;
  let recent = [];
  let turnCount = 0;
  let currentModel = "";
  let currentWorkspace = "";
  let keyConfigured = false;
  let activeTurn = 0;
  let followScroll = true;
  const FOLLOW_THRESHOLD = 72;

  function readableError(err) {
    const raw = (err && err.message) || (typeof err === "string" ? err : "");
    if (/API_KEY/i.test(raw) || (/is not set/i.test(raw) && /LLM/i.test(raw))) {
      return "未配置 THESEUS_LLM_API_KEY";
    }
    const cleaned = raw.replace(/THESEUS_[A-Z0-9_]+|PI_[A-Z0-9_]+/g, "配置");
    return cleaned || "请求失败。";
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
    if (!sentence || /^thr_[A-Za-z0-9_-]+$/.test(sentence)) return fallback || "新对话";
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
    titleEl.textContent = shortLabel(raw, fallback || (threadId ? "新对话" : "未打开对话"));
  }

  function showThreadId(id) {
    threadId = id || null;
    copyThreadIdEl.classList.toggle("hidden", !threadId);
    copyThreadIdEl.disabled = !threadId;
    copyThreadIdEl.setAttribute("aria-label", "复制 thread id");
    titleEl.title = threadId || "";
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

  function relativeTime(unix) {
    const n = Number(unix);
    if (!n) return "";
    const ms = n < 1e12 ? n * 1000 : n;
    const sec = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (sec < 45) return "刚刚";
    if (sec < 3600) return `${Math.floor(sec / 60)}分钟前`;
    if (sec < 86400) return `${Math.floor(sec / 3600)}小时前`;
    if (sec < 86400 * 30) return `${Math.floor(sec / 86400)}天前`;
    const d = new Date(ms);
    const month = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    return `${d.getFullYear()}-${month}-${day}`;
  }

  function setBusy(on, stopped) {
    busy = on;
    busyThreadId = on ? threadId : null;
    modelBusyEl.classList.toggle("on", on);
    sendEl.classList.toggle("hidden", on);
    stopEl.classList.toggle("hidden", !on);
    stoppedEl.classList.toggle("hidden", on || stopped !== true);
    const ready = Boolean(ws && ws.readyState === 1 && threadId && !busy && !approval);
    inputEl.disabled = !ready;
    sendEl.disabled = !ready;
    stopEl.disabled = !on;
    const connected = Boolean(ws && ws.readyState === 1 && !busy);
    newEl.disabled = !connected;
    emptyCtaEl.disabled = !connected;
    renderSessions();
  }

  function clearBusyChrome() {
    busy = false;
    busyThreadId = null;
    modelBusyEl.classList.remove("on");
    stopEl.classList.add("hidden");
    stopEl.disabled = true;
    sendEl.classList.remove("hidden");
    stoppedEl.classList.add("hidden");
    renderSessions();
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
    modelNameEl.textContent = name || "模型";
    modelVariantEl.textContent = variant;
    modelInputEl.value = currentModel;
  }

  function closeModelPop() {
    modelPopEl.classList.add("hidden");
    modelStripEl.setAttribute("aria-expanded", "false");
  }

  function openModelPop() {
    modelPopEl.classList.remove("hidden");
    modelStripEl.setAttribute("aria-expanded", "true");
    modelInputEl.value = currentModel;
    modelInputEl.focus();
    modelInputEl.select();
  }

  function applySettingsPayload(data) {
    if (data && data.model) currentModel = data.model;
    if (data && data.workspace) renderWorkspace(data.workspace);
    if (data && typeof data.keyConfigured === "boolean") keyConfigured = data.keyConfigured;
    if (data && data.baseUrl && prefsBaseEl) prefsBaseEl.value = data.baseUrl;
    if (prefsModelEl) prefsModelEl.value = currentModel;
    if (prefsKeyStatusEl) {
      prefsKeyStatusEl.textContent = keyConfigured ? "已配置" : "未配置";
    }
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
    prefsModelEl.value = currentModel;
    prefsWorkspaceEl.value = currentWorkspace;
    prefsKeyEl.value = "";
    prefsKeyStatusEl.textContent = keyConfigured ? "已配置" : "未配置";
    prefsBaseEl.focus();
  }

  function closePrefs() {
    document.body.classList.remove("prefs-open");
    prefsMaskEl.classList.add("hidden");
    prefsKeyEl.value = "";
  }

  async function savePrefs() {
    const payload = {
      baseUrl: prefsBaseEl.value.trim(),
      model: prefsModelEl.value.trim(),
      workspace: prefsWorkspaceEl.value.trim(),
    };
    const pasted = prefsKeyEl.value.trim();
    if (pasted) payload.key = pasted;
    prefsKeyEl.value = "";
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

  async function saveModel(raw) {
    const name = String(raw || "").trim();
    if (!name) return;
    try {
      const res = await fetch("/model", {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ model: name }),
      });
      const data = await res.json();
      if (data && data.model) currentModel = data.model;
      else currentModel = name;
    } catch {
      currentModel = name;
    }
    renderModel();
    closeModelPop();
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

  function nearBottom() {
    return threadEl.scrollHeight - threadEl.scrollTop - threadEl.clientHeight <= FOLLOW_THRESHOLD;
  }

  function followIfPinned() {
    if (!followScroll) return;
    threadEl.scrollTop = threadEl.scrollHeight;
  }

  function pinFollow() {
    followScroll = true;
    followIfPinned();
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

  function syncEmpty() {
    emptyEl.classList.toggle("hidden", Boolean(threadId) || logEl.children.length > 0);
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
      li.innerHTML = `<div class="tool-card"><button type="button" class="tool-head"><span class="tool-name">${escapeHtml(
        name
      )}</span><span class="tool-brief">${escapeHtml(
        brief
      )}</span></button><pre class="tool-body hidden">${escapeHtml(body)}</pre></div>`;
      li.querySelector(".tool-head").addEventListener("click", () => {
        li.querySelector(".tool-body").classList.toggle("hidden");
      });
    } else {
      li.classList.add("tool");
      li.textContent = type || "item";
    }
    syncEmpty();
    followIfPinned();
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
    followIfPinned();
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  function clearLog() {
    items.clear();
    logEl.innerHTML = "";
    turnCount = 0;
    activeTurn = 0;
    timelineEl.innerHTML = "";
    hideApproval();
    syncEmpty();
  }

  function hideApproval() {
    approval = null;
    approvalEl.classList.add("hidden");
    approvalToolEl.textContent = "";
    approvalSummaryEl.textContent = "";
    formEl.classList.remove("gated");
    composerInputEl.classList.remove("hidden");
    renderSessions();
    const ready = Boolean(ws && ws.readyState === 1 && threadId && !busy);
    inputEl.disabled = !ready;
    sendEl.disabled = !ready;
    stopEl.disabled = !busy;
  }

  function showApproval(params) {
    approval = params;
    approvalToolEl.textContent = params.tool || "tool";
    approvalSummaryEl.textContent =
      params.summary || briefArgs(params.arguments) || pretty(JSON.stringify(params.arguments || {}));
    approvalEl.classList.remove("hidden");
    formEl.classList.add("gated");
    composerInputEl.classList.add("hidden");
    approvalApproveEl.disabled = false;
    approvalRejectEl.disabled = false;
    renderSessions();
  }

  async function decideApproval(method) {
    if (!approval || !threadId) return;
    const callId = approval.callId;
    approvalApproveEl.disabled = true;
    approvalRejectEl.disabled = true;
    try {
      await rpc(method, { threadId, callId });
    } catch (err) {
      showBanner(readableError(err));
      approvalApproveEl.disabled = false;
      approvalRejectEl.disabled = false;
    }
  }

  function renderSessions() {
    sessionsEl.querySelectorAll(".session").forEach((n) => n.remove());
    sessionsEmptyEl.classList.toggle("hidden", recent.length > 0);
    for (const t of recent) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "session" + (t.id === threadId ? " active" : "");
      btn.dataset.id = t.id;
      const live = t.id === busyThreadId || Boolean(approval && approval.threadId === t.id);
      btn.innerHTML = `<span class="session-dot${
        live ? "" : " off"
      }"></span><span class="session-main"><span class="session-title">${escapeHtml(
        shortLabel(t.preview, "新对话")
      )}</span><span class="session-time">${escapeHtml(relativeTime(t.updatedAt))}</span></span>`;
      btn.addEventListener("click", () => {
        railEl.classList.remove("open");
        resumeThread(t.id).catch((err) => {
          showBanner(readableError(err) || "打不开这个线程。");
        });
      });
      sessionsEl.appendChild(btn);
    }
  }

  function applyThread(result) {
    const thread = result && result.thread;
    if (!thread) return;
    showThreadId(thread.id);
    threadPreview = shortLabel(thread.preview, "新对话");
    setTitle(thread.preview, "新对话");
    renderWorkspace(thread.cwd || currentWorkspace);
    clearLog();
    for (const turn of thread.turns || []) {
      turnCount += 1;
      addTurnMark();
      for (const item of turn.items || []) {
        if (item && item.id) upsert(item);
      }
    }
    setBusy(false);
    pinFollow();
    inputEl.focus();
    renderSessions();
  }

  function onNotify(msg) {
    switch (msg.method) {
      case "thread/started": {
        const thread = msg.params && msg.params.thread;
        if (!thread) return;
        showThreadId(thread.id);
        threadPreview = shortLabel(thread.preview, "新对话");
        setTitle(thread.preview, "新对话");
        if (thread.cwd) renderWorkspace(thread.cwd);
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
            threadPreview = shortLabel(text, "新对话");
            setTitle(text, "新对话");
          }
        }
        break;
      }
      case "item/agentMessage/delta": {
        const id = msg.params && msg.params.itemId;
        const delta = (msg.params && msg.params.delta) || "";
        if (!id) return;
        const prev = items.get(id) || { type: "agentMessage", id, text: "" };
        prev.text = (prev.text || "") + delta;
        upsert(prev);
        break;
      }
      case "item/tool/approval/request": {
        if (msg.params) showApproval(msg.params);
        break;
      }
      case "item/tool/approval/resolved": {
        hideApproval();
        break;
      }
      case "turn/completed": {
        const turn = msg.params && msg.params.turn;
        if (turn && turn.status === "failed") {
          showBanner("这一轮失败了。服务端已经收口，没有半截助手消息。");
        }
        const stopped = Boolean(
          turn && (turn.status === "aborted" || turn.status === "interrupted")
        );
        hideApproval();
        setBusy(false, stopped);
        loadRecent();
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
      result = await rpc("thread/list", { limit: 20 });
    } catch (err) {
      showBanner(readableError(err));
      return;
    }
    recent = (result && result.threads) || [];
    renderSessions();
  }

  async function startThread() {
    showBanner("");
    clearLog();
    showThreadId(null);
    setTitle("新对话", "新对话");
    const params = {};
    if (currentModel) params.model = currentModel;
    if (currentWorkspace) params.cwd = currentWorkspace;
    const result = await rpc("thread/start", params);
    applyThread(result);
    await loadRecent();
  }

  async function resumeThread(id) {
    if (!id) return;
    showBanner("");
    const result = await rpc("thread/resume", { threadId: id });
    applyThread(result);
  }

  async function sendTurn(text) {
    if (!threadId || busy || approval) return;
    showBanner("");
    setTitle(text, "新对话");
    threadPreview = shortLabel(text, "新对话");
    pinFollow();
    setBusy(true);
    try {
      await rpc("turn/start", {
        threadId,
        input: [{ type: "text", text }],
      });
    } catch (err) {
      showBanner(readableError(err), /API_KEY|is not set/i.test((err && err.message) || "") ? "warn" : undefined);
      setBusy(false);
    }
  }

  async function stopTurn() {
    if (!threadId || !busy) return;
    try {
      await rpc("turn/interrupt", { threadId });
    } catch (err) {
      showBanner(readableError(err));
      setBusy(false);
    }
  }

  function connect() {
    setTitle("未打开对话", "未打开对话");
    showThreadId(null);
    const proto = location.protocol === "https:" ? "wss" : "ws";
    ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.onopen = () => {
      setBusy(false);
      loadRecent().catch(() => {});
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
    sendTurn(text);
  });

  inputEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      formEl.requestSubmit();
    }
  });

  approvalApproveEl.addEventListener("click", () => {
    decideApproval("tool/approve");
  });
  approvalRejectEl.addEventListener("click", () => {
    decideApproval("tool/reject");
  });

  stopEl.addEventListener("click", () => {
    stopTurn();
  });

  function startFromCta() {
    startThread().catch((err) => {
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
  document.querySelectorAll(".prefs-preset").forEach((btn) => {
    btn.addEventListener("click", () => {
      prefsModelEl.value = btn.dataset.model || "";
    });
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !prefsMaskEl.classList.contains("hidden")) {
      closePrefs();
    }
  });

  toggleRailEl.addEventListener("click", () => {
    railEl.classList.toggle("open");
  });

  modelStripEl.addEventListener("click", (e) => {
    e.preventDefault();
    if (modelPopEl.classList.contains("hidden")) openModelPop();
    else closeModelPop();
  });

  modelInputEl.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      saveModel(modelInputEl.value);
    } else if (e.key === "Escape") {
      closeModelPop();
    }
  });

  modelPopEl.querySelectorAll(".model-choice").forEach((btn) => {
    btn.addEventListener("click", () => saveModel(btn.dataset.model));
  });

  document.addEventListener("click", (e) => {
    if (!modelPopEl.classList.contains("hidden")) {
      const wrap = modelStripEl.closest(".model-wrap");
      if (wrap && !wrap.contains(e.target)) closeModelPop();
    }
  });

  threadEl.addEventListener(
    "scroll",
    () => {
      followScroll = nearBottom();
      syncActiveTurn();
    },
    { passive: true }
  );

  copyThreadIdEl.addEventListener("click", () => {
    if (!threadId) return;
    copyText(threadId)
      .then(() => {
        copyThreadIdEl.setAttribute("aria-label", "已复制");
        setTimeout(() => {
          if (copyThreadIdEl.getAttribute("aria-label") === "已复制") {
            copyThreadIdEl.setAttribute("aria-label", "复制 thread id");
          }
        }, 1200);
      })
      .catch(() => {
        copyThreadIdEl.setAttribute("aria-label", "复制失败");
        setTimeout(() => {
          if (copyThreadIdEl.getAttribute("aria-label") === "复制失败") {
            copyThreadIdEl.setAttribute("aria-label", "复制 thread id");
          }
        }, 1200);
      });
  });

  loadModel();
  connect();
})();
