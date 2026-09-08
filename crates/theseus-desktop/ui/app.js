/* Thin JSON-RPC client. Projects Thread / Turn / Item + one gate. No loop. */
(() => {
  const $ = (id) => document.getElementById(id);
  const logEl = $("log");
  const emptyEl = $("empty");
  const bannerEl = $("banner");
  const titleEl = $("title");
  const metaEl = $("meta");
  const runEl = $("run-state");
  const threadIdBox = $("thread-id-box");
  const threadIdEl = $("thread-id");
  const copyThreadIdEl = $("copy-thread-id");
  const inputEl = $("input");
  const sendEl = $("send");
  const newEl = $("new-thread");
  const sessionsEl = $("sessions");
  const sessionsEmptyEl = $("sessions-empty");
  const formEl = $("composer");
  const approvalEl = $("approval");
  const approvalToolEl = $("approval-tool");
  const approvalSummaryEl = $("approval-summary");
  const approvalApproveEl = $("approval-approve");
  const approvalRejectEl = $("approval-reject");
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

  let ws = null;
  let nextId = 1;
  const pending = new Map();
  let threadId = null;
  let threadPreview = "";
  let busy = false;
  const items = new Map();
  let approval = null;
  let recent = [];
  let turnCount = 0;
  let currentModel = "";
  let activeTurn = 0;

  function readableError(err) {
    const raw = (err && err.message) || (typeof err === "string" ? err : "");
    if (/API_KEY/i.test(raw) || (/is not set/i.test(raw) && /LLM/i.test(raw))) {
      return "未配置 THESEUS_LLM_API_KEY";
    }
    const cleaned = raw.replace(/THESEUS_[A-Z0-9_]+|PI_[A-Z0-9_]+/g, "配置");
    return cleaned || "请求失败。";
  }

  function showThreadId(id) {
    threadId = id || null;
    if (threadId) {
      threadIdEl.textContent = threadId;
      threadIdEl.title = threadId;
      threadIdBox.classList.remove("hidden");
      copyThreadIdEl.disabled = false;
      copyThreadIdEl.textContent = "复制 id";
    } else {
      threadIdEl.textContent = "";
      threadIdEl.title = "";
      threadIdBox.classList.add("hidden");
      copyThreadIdEl.disabled = true;
      copyThreadIdEl.textContent = "复制 id";
    }
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
    runEl.classList.toggle("hidden", !on);
    modelBusyEl.classList.toggle("on", on);
    const ready = Boolean(ws && ws.readyState === 1 && threadId && !busy && !approval);
    inputEl.disabled = !ready;
    sendEl.disabled = !ready;
    const connected = Boolean(ws && ws.readyState === 1 && !busy);
    newEl.disabled = !connected;
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

  async function loadModel() {
    try {
      const res = await fetch("/model");
      const data = await res.json();
      currentModel = (data && data.model) || "";
    } catch {
      currentModel = currentModel || "";
    }
    renderModel();
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

  function relativeTime(ts) {
    if (!ts) return "";
    const sec = ts > 1e12 ? Math.floor(ts / 1000) : ts;
    const d = Date.now() / 1000 - sec;
    if (d < 45) return "刚刚";
    if (d < 3600) return `${Math.floor(d / 60)}分钟前`;
    if (d < 86400) return `${Math.floor(d / 3600)}小时前`;
    return `${Math.floor(d / 86400)}天前`;
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
      li.innerHTML = `<span class="who">你</span><div class="bubble">${escapeHtml(text)}</div>`;
    } else if (type === "agentMessage") {
      li.classList.add("agent");
      li.innerHTML = `<span class="who">助手</span><div class="bubble">${escapeHtml(item.text || "")}</div>`;
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
    emptyEl.classList.toggle("hidden", logEl.children.length > 0);
    li.scrollIntoView({ block: "end" });
  }

  function addTurnMark(label) {
    const n = turnCount;
    const li = document.createElement("li");
    li.className = "turn-mark";
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "turn-anchor";
    btn.id = `turn-${n}`;
    btn.dataset.turn = String(n);
    btn.textContent = label || `回合 ${n}`;
    btn.addEventListener("click", () => scrollToTurn(n));
    li.appendChild(btn);
    logEl.appendChild(li);
    emptyEl.classList.add("hidden");
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
    logEl.innerHTML = "";
    emptyEl.classList.remove("hidden");
    turnCount = 0;
    activeTurn = 0;
    timelineEl.innerHTML = "";
    hideApproval();
  }

  function hideApproval() {
    approval = null;
    approvalEl.classList.add("hidden");
    approvalToolEl.textContent = "";
    approvalSummaryEl.textContent = "";
    formEl.classList.remove("hidden");
    renderSessions();
    const ready = Boolean(ws && ws.readyState === 1 && threadId && !busy);
    inputEl.disabled = !ready;
    sendEl.disabled = !ready;
  }

  function showApproval(params) {
    approval = params;
    approvalToolEl.textContent = params.tool || "tool";
    approvalSummaryEl.textContent =
      params.summary || briefArgs(params.arguments) || pretty(JSON.stringify(params.arguments || {}));
    approvalEl.classList.remove("hidden");
    formEl.classList.add("hidden");
    approvalApproveEl.disabled = false;
    approvalRejectEl.disabled = false;
    approvalEl.scrollIntoView({ block: "end" });
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
      const waiting = Boolean(approval && approval.threadId === t.id);
      const when = relativeTime(t.updatedAt);
      btn.innerHTML = `<span class="session-dot${
        waiting ? "" : " off"
      }"></span><span class="session-preview">${escapeHtml(
        t.preview || t.id
      )}</span><span class="session-when">${escapeHtml(when)}</span>`;
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
    threadPreview = thread.preview || thread.id;
    const cwd = thread.cwd || "工作区";
    titleEl.textContent = threadPreview;
    metaEl.textContent = cwd;
    clearLog();
    for (const turn of thread.turns || []) {
      turnCount += 1;
      addTurnMark(`回合 ${turnCount}`);
      for (const item of turn.items || []) {
        if (item && item.id) upsert(item);
      }
    }
    setBusy(false);
    inputEl.focus();
    renderSessions();
  }

  function onNotify(msg) {
    switch (msg.method) {
      case "thread/started": {
        const thread = msg.params && msg.params.thread;
        if (!thread) return;
        showThreadId(thread.id);
        threadPreview = thread.preview || thread.id;
        titleEl.textContent = threadPreview;
        metaEl.textContent = thread.cwd || "工作区";
        break;
      }
      case "turn/started": {
        turnCount += 1;
        addTurnMark(`回合 ${turnCount}`);
        break;
      }
      case "item/started":
      case "item/completed": {
        const item = msg.params && msg.params.item;
        if (item && item.id) upsert(item);
        if (item && item.type === "userMessage") {
          const text = (item.content || []).map((c) => c.text || "").join("\n");
          if (text) {
            threadPreview = text.slice(0, 80);
            titleEl.textContent = threadPreview;
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
        } else if (turn && turn.status === "aborted") {
          showBanner("这一轮中止了。", "warn");
        }
        hideApproval();
        setBusy(false);
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
    titleEl.textContent = "新对话";
    const params = currentModel ? { model: currentModel } : {};
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

  function connect() {
    titleEl.textContent = "未打开对话";
    showThreadId(null);
    metaEl.textContent = "正在连接…";
    const proto = location.protocol === "https:" ? "wss" : "ws";
    ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.onopen = () => {
      metaEl.textContent = "已连接 · 选一个线程或开新对话";
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
      setBusy(true);
      inputEl.disabled = true;
      sendEl.disabled = true;
      newEl.disabled = true;
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

  newEl.addEventListener("click", () => {
    startThread().catch((err) => {
      showBanner(readableError(err) || "开不了新对话。");
    });
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

  threadEl.addEventListener("scroll", syncActiveTurn, { passive: true });

  copyThreadIdEl.addEventListener("click", () => {
    if (!threadId) return;
    copyText(threadId)
      .then(() => {
        copyThreadIdEl.textContent = "已复制";
        setTimeout(() => {
          if (copyThreadIdEl.textContent === "已复制") {
            copyThreadIdEl.textContent = "复制 id";
          }
        }, 1200);
      })
      .catch(() => {
        copyThreadIdEl.textContent = "复制失败";
        setTimeout(() => {
          if (copyThreadIdEl.textContent === "复制失败") {
            copyThreadIdEl.textContent = "复制 id";
          }
        }, 1200);
      });
  });

  loadModel();
  connect();
})();
