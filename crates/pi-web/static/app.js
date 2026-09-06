/* Thin JSON-RPC client. Renders Thread / Turn / Item + delta only. */
(() => {
  const $ = (id) => document.getElementById(id);
  const logEl = $("log");
  const emptyEl = $("empty");
  const bannerEl = $("banner");
  const metaEl = $("meta");
  const inputEl = $("input");
  const sendEl = $("send");
  const newEl = $("new-thread");
  const recentEl = $("recent");
  const formEl = $("composer");
  const approvalEl = $("approval");
  const approvalToolEl = $("approval-tool");
  const approvalSummaryEl = $("approval-summary");
  const approvalApproveEl = $("approval-approve");
  const approvalRejectEl = $("approval-reject");

  let ws = null;
  let nextId = 1;
  const pending = new Map();
  let threadId = null;
  let busy = false;
  const items = new Map();
  let approval = null;

  function showBanner(text) {
    if (!text) {
      bannerEl.classList.add("hidden");
      bannerEl.textContent = "";
      return;
    }
    bannerEl.textContent = text;
    bannerEl.classList.remove("hidden");
  }

  function setBusy(on) {
    busy = on;
    const ready = Boolean(ws && ws.readyState === 1 && threadId && !busy);
    inputEl.disabled = !ready;
    sendEl.disabled = !ready;
    const connected = Boolean(ws && ws.readyState === 1 && !busy);
    newEl.disabled = !connected;
    recentEl.disabled = !connected;
  }

  function pretty(raw) {
    try {
      return JSON.stringify(JSON.parse(raw), null, 2);
    } catch {
      return raw || "";
    }
  }

  function upsert(item) {
    items.set(item.id, item);
    let li = document.getElementById(`item-${item.id}`);
    if (!li) {
      li = document.createElement("li");
      li.id = `item-${item.id}`;
      li.className = "card";
      logEl.appendChild(li);
    }
    const type = item.type;
    li.classList.remove("user", "agent", "tool", "error");
    if (type === "userMessage") {
      li.classList.add("user");
      const text = (item.content || []).map((c) => c.text || "").join("\n");
      li.innerHTML = `<span class="kind">你</span>${escapeHtml(text)}`;
    } else if (type === "agentMessage") {
      li.classList.add("agent");
      li.innerHTML = `<span class="kind">助手</span>${escapeHtml(item.text || "")}`;
    } else if (type === "toolCall") {
      li.classList.add("tool");
      li.innerHTML = `<span class="kind">tool ${escapeHtml(item.name || "")}</span>${escapeHtml(
        pretty(item.arguments || "")
      )}`;
    } else if (type === "toolResult") {
      li.classList.add("tool");
      if (item.isError) li.classList.add("error");
      li.innerHTML = `<span class="kind">${item.isError ? "tool error" : "tool result"}</span>${escapeHtml(
        item.output || ""
      )}`;
    } else {
      li.classList.add("tool");
      li.textContent = type || "item";
    }
    emptyEl.classList.toggle("hidden", logEl.children.length > 0);
    li.scrollIntoView({ block: "end" });
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
    hideApproval();
  }

  function hideApproval() {
    approval = null;
    approvalEl.classList.add("hidden");
    approvalToolEl.textContent = "";
    approvalSummaryEl.textContent = "";
  }

  function showApproval(params) {
    approval = params;
    approvalToolEl.textContent = params.tool || "tool";
    approvalSummaryEl.textContent = params.summary || pretty(JSON.stringify(params.arguments || {}));
    approvalEl.classList.remove("hidden");
    approvalApproveEl.disabled = false;
    approvalRejectEl.disabled = false;
  }

  async function decideApproval(method) {
    if (!approval || !threadId) return;
    const callId = approval.callId;
    approvalApproveEl.disabled = true;
    approvalRejectEl.disabled = true;
    try {
      await rpc(method, { threadId, callId });
    } catch (err) {
      showBanner((err && err.message) || method + " 失败");
    }
  }

  function applyThread(result) {
    const thread = result && result.thread;
    if (!thread) return;
    threadId = thread.id;
    const model = result.model || "";
    const cwd = thread.cwd || "进程工作区";
    metaEl.textContent = [thread.id, model, cwd].filter(Boolean).join(" · ");
    clearLog();
    for (const turn of thread.turns || []) {
      for (const item of turn.items || []) {
        if (item && item.id) upsert(item);
      }
    }
    recentEl.value = thread.id;
    setBusy(false);
    inputEl.focus();
  }

  function onNotify(msg) {
    switch (msg.method) {
      case "thread/started": {
        const thread = msg.params && msg.params.thread;
        if (!thread) return;
        threadId = thread.id;
        const cwd = thread.cwd || "进程工作区";
        metaEl.textContent = `${thread.id} · ${cwd}`;
        break;
      }
      case "item/started":
      case "item/completed": {
        const item = msg.params && msg.params.item;
        if (item && item.id) upsert(item);
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
          showBanner("这一轮失败了。");
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
    } catch {
      return;
    }
    const threads = (result && result.threads) || [];
    const keep = threadId;
    recentEl.innerHTML = "";
    const blank = document.createElement("option");
    blank.value = "";
    blank.textContent = threads.length ? "选择对话" : "没有最近对话";
    recentEl.appendChild(blank);
    for (const t of threads) {
      const opt = document.createElement("option");
      opt.value = t.id;
      opt.textContent = t.preview || t.id;
      if (t.id === keep) opt.selected = true;
      recentEl.appendChild(opt);
    }
  }

  async function startThread() {
    showBanner("");
    clearLog();
    threadId = null;
    const result = await rpc("thread/start", {});
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
    if (!threadId || busy) return;
    showBanner("");
    setBusy(true);
    try {
      await rpc("turn/start", {
        threadId,
        input: [{ type: "text", text }],
      });
    } catch (err) {
      showBanner((err && err.message) || "turn/start 失败");
      setBusy(false);
    }
  }

  function connect() {
    metaEl.textContent = "正在连接…";
    const proto = location.protocol === "https:" ? "wss" : "ws";
    ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.onopen = () => {
      metaEl.textContent = "已连接";
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
      recentEl.disabled = true;
      showBanner("连接断开，请刷新页面。");
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
      showBanner((err && err.message) || "thread/start 失败");
    });
  });

  recentEl.addEventListener("change", () => {
    const id = recentEl.value;
    if (!id) return;
    resumeThread(id).catch((err) => {
      showBanner((err && err.message) || "thread/resume 失败");
    });
  });

  connect();
})();
