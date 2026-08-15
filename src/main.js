// DeepSeek Harness 桌面端 —— 前端控制逻辑（Vanilla JS，无框架）
// 通过 window.__TAURI__（withGlobalTauri 注入）调用 Rust 后端命令。
(() => {
  "use strict";

  const TAURI = window.__TAURI__;
  const invoke = TAURI?.core?.invoke;
  const appWindow = TAURI?.window?.getCurrentWindow?.();

  const WEB_PORT = 3080;
  const WEB_URL = "http://127.0.0.1:3080";

  const $ = (id) => document.getElementById(id);
  const els = {
    statusDot: $("status-dot"),
    statusTitle: $("status-title"),
    statusDetail: $("status-detail"),
    chipNode: $("chip-node"),
    chipDsh: $("chip-dsh"),
    chipWebview2: $("chip-webview2"),
    btnEnvRefresh: $("btn-env-refresh"),
    btnStart: $("btn-start"),
    btnStop: $("btn-stop"),
    btnOpen: $("btn-open"),
    btnReload: $("btn-reload"),
    btnLog: $("btn-log"),
    chkAutoload: $("chk-autoload"),
    banner: $("banner"),
    bannerText: $("banner-text"),
    bannerCmd: $("banner-cmd"),
    frame: $("web-frame"),
    placeholder: $("web-placeholder"),
    placeholderText: document.querySelector("#web-placeholder p"),
    btnLoadWeb: $("btn-load-web"),
    badge: $("web-badge"),
    logView: $("log-view"),
  };

  const state = {
    env: null,
    portInUse: false,
    owned: false,
    busy: false,        // 正在启动/停止
    busyAction: "启动",
    inTauri: !!invoke,
    logOpen: false,
    autoLoad: localStorage.getItem("autoload") !== "0", // 默认开（符合任务书：启动后自动内嵌）
    forceLoad: false,   // 关闭自动加载后，用户手动点过一次加载
    pollTimer: null,
  };

  // ---------------- 渲染 ----------------

  function setStatus(kind, title, detail) {
    els.statusDot.className = "status-dot " + kind;
    if (title) els.statusTitle.textContent = title;
    if (detail) els.statusDetail.textContent = detail;
  }

  function showBanner(text, cmdText) {
    els.banner.classList.remove("hidden");
    els.bannerText.textContent = text;
    els.bannerCmd.textContent = cmdText || "";
    els.bannerCmd.classList.toggle("hidden", !cmdText);
  }
  function hideBanner() { els.banner.classList.add("hidden"); }

  function setChip(chip, ok, label, hint) {
    chip.querySelector(".chip-icon").textContent = ok ? "✓" : "✗";
    chip.querySelector(".chip-icon").className = "chip-icon " + (ok ? "ok" : "bad");
    chip.querySelector(".chip-label").textContent = label;
    chip.classList.toggle("bad", !ok);
    chip.title = hint || "";
  }

  function render() {
    const { portInUse, owned, busy, busyAction, env } = state;

    // 状态灯与文案
    if (busy) {
      setStatus("starting", `正在${busyAction}服务…`, busyAction === "停止" ? "正在终止 dsh 进程树…" : "等待 3080 端口就绪…");
    } else if (portInUse && owned) {
      setStatus("running", "服务运行中", `Harness Web UI：${WEB_URL}（由本应用托管）`);
    } else if (portInUse) {
      setStatus("external", "Harness 可能已在运行", `检测到 ${WEB_URL} 端口已被占用，已直接加载现有实例，未重复启动。`);
    } else {
      setStatus("stopped", "服务已停止", `点击「启动服务」运行 dsh web --port ${WEB_PORT}（${WEB_URL}）`);
    }

    const envOk = !!(env && env.node && env.dsh);
    els.btnStart.disabled = !state.inTauri || !envOk || portInUse || busy;
    els.btnStop.disabled = !state.inTauri || busy || !portInUse || !owned;
    els.btnOpen.disabled = !state.inTauri || !portInUse;

    // 内嵌 Web UI（受「自动加载」开关与手动加载控制）
    const shouldShowFrame = portInUse && (state.autoLoad || state.forceLoad);
    els.btnReload.disabled = !shouldShowFrame;

    if (shouldShowFrame) {
      els.placeholder.classList.add("hidden");
      els.frame.classList.remove("hidden");
      if (!els.frame.src || !els.frame.src.includes("127.0.0.1")) {
        els.frame.src = WEB_URL;
      }
      els.badge.textContent = "已连接";
      els.badge.className = "web-badge on";
    } else {
      els.frame.classList.add("hidden");
      els.frame.src = "about:blank"; // 释放渲染进程，回收内存
      els.placeholder.classList.remove("hidden");
      if (portInUse) {
        els.placeholderText.textContent = "服务运行中（未自动加载 Web 界面，以节省内存）";
        els.btnLoadWeb.classList.remove("hidden");
      } else {
        els.placeholderText.textContent = "Web 界面将在服务启动后自动加载";
        els.btnLoadWeb.classList.add("hidden");
        state.forceLoad = false;
      }
      els.badge.textContent = "未连接";
      els.badge.className = "web-badge";
    }
  }

  // ---------------- 数据刷新 ----------------

  async function refresh() {
    if (!invoke) return;
    try {
      const s = await invoke("get_status");
      state.portInUse = s.portInUse;
      state.owned = s.owned;
    } catch (e) {
      console.error("get_status 失败", e);
    }
    render();
  }

  async function refreshEnv() {
    if (!invoke) {
      setChip(els.chipNode, false, "Node.js", "请在 Tauri 桌面应用中运行");
      setChip(els.chipDsh, false, "dsh", "请在 Tauri 桌面应用中运行");
      setChip(els.chipWebview2, true, "WebView2", "Windows 11 内置");
      render();
      return;
    }
    try {
      const env = await invoke("get_env_info");
      state.env = env;
      setChip(els.chipNode, !!env.node, "Node.js", env.node || "未检测到系统 Node.js");
      setChip(els.chipDsh, !!env.dsh, "dsh", env.dsh || "未检测到全局 dsh，请运行：npm install -g @deepseek/dsh");
      setChip(els.chipWebview2, true, "WebView2", "Windows 11 已内置 WebView2 运行时");
      if (!env.node || !env.dsh) {
        const missing = [];
        if (!env.node) missing.push("Node.js");
        if (!env.dsh) missing.push("全局 dsh");
        showBanner(`缺少运行环境：${missing.join("、")}`, env.dsh ? undefined : "npm install -g @deepseek/dsh");
      } else {
        hideBanner();
      }
      render();
    } catch (e) {
      console.error("get_env_info 失败", e);
    }
  }

  // ---------------- 启停流程 ----------------

  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  async function waitPortUp(timeoutMs) {
    const t0 = Date.now();
    while (Date.now() - t0 < timeoutMs) {
      await refresh();
      if (state.portInUse) return true;
      await sleep(600);
    }
    return state.portInUse;
  }

  async function waitPortDown(timeoutMs) {
    const t0 = Date.now();
    while (Date.now() - t0 < timeoutMs) {
      await refresh();
      if (!state.portInUse) return true;
      await sleep(400);
    }
    return !state.portInUse;
  }

  async function startService() {
    if (!invoke || state.busy) return;
    hideBanner();
    state.busy = true;
    state.busyAction = "启动";
    render();
    try {
      const res = await invoke("start_service");
      if (res && res.status === "alreadyRunning") {
        await refresh();
      } else {
        await waitPortUp(30000);
      }
    } catch (e) {
      console.error("启动失败", e);
      const msg = e?.message || String(e);
      const code = e?.code;
      if (code === "NodeMissing") {
        showBanner("未检测到系统 Node.js，请先安装：", "https://nodejs.org/");
      } else if (code === "DshMissing") {
        showBanner("未检测到全局 dsh 命令，请安装：", "npm install -g @deepseek/dsh");
      } else {
        showBanner("服务启动失败", msg);
      }
      setStatus("error", "启动失败", msg);
    } finally {
      state.busy = false;
      await refresh();
    }
  }

  async function stopService() {
    if (!invoke || state.busy) return;
    state.busy = true;
    state.busyAction = "停止";
    render();
    try {
      await invoke("stop_service");
      await waitPortDown(10000);
    } catch (e) {
      console.error("停止失败", e);
    } finally {
      state.busy = false;
      await refresh();
    }
  }

  // ---------------- 其它操作 ----------------

  async function openExternal() {
    if (!invoke) return;
    try { await invoke("open_web_window"); } catch (e) { console.error("打开独立窗口失败", e); }
  }

  function reloadFrame() {
    els.frame.src = WEB_URL;
  }

  async function toggleLog() {
    state.logOpen = !state.logOpen;
    els.logView.classList.toggle("hidden", !state.logOpen);
    if (state.logOpen && invoke) {
      try {
        els.logView.textContent = (await invoke("read_log")) || "（暂无日志，启动服务后写入）";
      } catch {
        els.logView.textContent = "（无法读取日志）";
      }
    }
  }

  // ---------------- 轮询（窗口隐藏时暂停，节省开销） ----------------

  function startPolling() {
    state.pollTimer = setInterval(() => {
      if (!document.hidden) refresh();
    }, 3000);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) refresh();
    });
  }

  // ---------------- 窗口控制 ----------------

  function initControls() {
    if (!appWindow) {
      document.querySelectorAll(".tb-btn").forEach((b) => { b.disabled = true; });
      return;
    }
    $("btn-min").addEventListener("click", () => appWindow.minimize());
    $("btn-max").addEventListener("click", async () => {
      const max = await appWindow.isMaximized();
      if (max) await appWindow.unmaximize();
      else await appWindow.maximize();
    });
    $("btn-close").addEventListener("click", () => appWindow.close()); // Rust 侧：隐藏到托盘
  }

  // ---------------- 初始化 ----------------

  els.btnStart.addEventListener("click", startService);
  els.btnStop.addEventListener("click", stopService);
  els.btnOpen.addEventListener("click", openExternal);
  els.btnReload.addEventListener("click", reloadFrame);
  els.btnLog.addEventListener("click", toggleLog);
  els.btnEnvRefresh.addEventListener("click", refreshEnv);
  els.btnLoadWeb.addEventListener("click", () => {
    state.forceLoad = true;
    reloadFrame();
    render();
  });

  // 自动加载开关（持久化到 localStorage）
  els.chkAutoload.checked = state.autoLoad;
  els.chkAutoload.addEventListener("change", () => {
    state.autoLoad = els.chkAutoload.checked;
    localStorage.setItem("autoload", state.autoLoad ? "1" : "0");
    state.forceLoad = false; // 关闭时立刻释放已加载的渲染进程
    render();
  });

  initControls();

  if (!state.inTauri) {
    showBanner("当前不在 Tauri 桌面环境中运行", "请使用 npm run tauri dev 启动应用");
    render();
    return;
  }

  refreshEnv();
  refresh();
  startPolling();
})();
