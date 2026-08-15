// DeepSeek Harness 桌面端 —— 前端控制逻辑（Vanilla JS，无框架）
// 极简版：顶栏（状态 + 启停）+ 全屏 Web 界面，无多余工具。
// 通过 window.__TAURI__（withGlobalTauri 注入）调用 Rust 后端命令。
(() => {
  "use strict";

  const TAURI = window.__TAURI__;
  const invoke = TAURI?.core?.invoke;
  const appWindow = TAURI?.window?.getCurrentWindow?.();

  const WEB_URL = "http://127.0.0.1:3080";

  const $ = (id) => document.getElementById(id);
  const els = {
    statusDot: $("status-dot"),
    statusText: $("status-text"),
    btnToggle: $("btn-toggle"),
    btnFullscreen: $("btn-fullscreen"),
    banner: $("banner"),
    bannerText: $("banner-text"),
    bannerCmd: $("banner-cmd"),
    btnRetry: $("btn-retry"),
    frame: $("web-frame"),
    placeholder: $("web-placeholder"),
  };

  const state = {
    env: null,
    portInUse: false,
    owned: false,
    busy: false,        // 正在启动/停止
    busyAction: "启动",
    inTauri: !!invoke,
  };

  // ---------------- 状态渲染 ----------------

  function setStatus(kind, text, detail) {
    els.statusDot.className = "status-dot " + kind;
    els.statusText.textContent = text;
    els.statusText.title = detail || "";
  }

  function showBanner(text, cmdText, opts = {}) {
    els.banner.className = "banner" + (opts.info ? " info" : "");
    els.bannerText.textContent = text;
    els.bannerCmd.textContent = cmdText || "";
    els.bannerCmd.classList.toggle("hidden", !cmdText);
    els.btnRetry.classList.toggle("hidden", !opts.retry);
    els.btnRetry.onclick = opts.retry || null;
  }
  function hideBanner() { els.banner.classList.add("hidden"); }

  function statusDetail() {
    const { portInUse, owned, busy, busyAction } = state;
    if (busy) return busyAction === "停止" ? "正在终止 dsh 进程树…" : "等待 3080 端口就绪…";
    if (portInUse && owned) return `dsh web 服务由本应用托管（${WEB_URL}）`;
    if (portInUse) return `检测到 ${WEB_URL} 已被占用，未重复启动，已直接加载现有实例`;
    return `点击「启动服务」运行 dsh web --port 3080（${WEB_URL}）`;
  }

  function render() {
    const { portInUse, owned, busy, busyAction, env } = state;

    // 状态文字 + 圆点
    if (busy) {
      setStatus("starting", `正在${busyAction}…`, statusDetail());
    } else if (portInUse && owned) {
      setStatus("running", "服务运行中", statusDetail());
    } else if (portInUse) {
      setStatus("external", "服务运行中", statusDetail());
    } else {
      setStatus("stopped", "服务已停止", statusDetail());
    }

    // 启停切换按钮（唯一主操作）
    const envOk = !!(env && env.node && env.dsh);
    if (portInUse) {
      els.btnToggle.textContent = "停止服务";
      els.btnToggle.className = "btn-toggle stop";
      els.btnToggle.disabled = !state.inTauri || busy || !owned;
    } else {
      els.btnToggle.textContent = "启动服务";
      els.btnToggle.className = "btn-toggle start";
      els.btnToggle.disabled = !state.inTauri || busy || !envOk;
    }

    // Web 主导区：运行中自动加载
    if (portInUse) {
      els.placeholder.classList.add("hidden");
      els.frame.classList.remove("hidden");
      if (!els.frame.src || !els.frame.src.includes("127.0.0.1")) {
        els.frame.src = WEB_URL;
      }
    } else {
      els.frame.classList.add("hidden");
      els.frame.src = "about:blank"; // 释放渲染进程，回收内存
      els.placeholder.classList.remove("hidden");
    }
  }

  // ---------------- 环境检测 ----------------

  async function refreshEnv() {
    if (!invoke) {
      showBanner("当前不在 Tauri 桌面环境中运行", "请使用 npm run tauri dev 启动应用");
      render();
      return;
    }
    try {
      const env = await invoke("get_env_info");
      state.env = env;
      if (!env.node || !env.dsh) {
        const missing = [];
        if (!env.node) missing.push("Node.js");
        if (!env.dsh) missing.push("全局 dsh");
        showBanner(
          `缺少运行环境：${missing.join("、")}`,
          env.dsh ? undefined : "npm install -g @deepseek/dsh",
          { retry: refreshEnv }
        );
      } else {
        hideBanner();
      }
      render();
    } catch (e) {
      console.error("get_env_info 失败", e);
    }
  }

  // ---------------- 全屏按钮 ----------------

  function updateFsUi(isFs) {
    document.getElementById("app").classList.toggle("fs", isFs); // 全屏时隐藏顶栏（CSS）
    if (!els.btnFullscreen) return;
    els.btnFullscreen.querySelector(".fs-enter").classList.toggle("hidden", isFs);
    els.btnFullscreen.querySelector(".fs-exit").classList.toggle("hidden", !isFs);
    els.btnFullscreen.title = isFs ? "退出全屏" : "全屏";
  }

  async function toggleFullscreen() {
    if (!appWindow) return;
    try {
      const isFs = await appWindow.isFullscreen();
      await appWindow.setFullscreen(!isFs);
      updateFsUi(!isFs);
    } catch (e) {
      console.error("切换全屏失败", e);
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
    // 同步全屏状态（按钮图标 + 顶栏隐藏），例如其它方式进入/退出全屏时保持一致
    if (appWindow) {
      try {
        const isFs = await appWindow.isFullscreen();
        updateFsUi(isFs);
      } catch (e) { /* 忽略 */ }
    }
    render();
  }

  function startPolling() {
    setInterval(() => {
      if (!document.hidden) refresh();
    }, 3000);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) refresh();
    });
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
        showBanner(
          "检测到 3080 端口已被占用，未重复启动，已直接加载现有实例。",
          undefined,
          { info: true }
        );
        await refresh();
      } else {
        await waitPortUp(30000);
      }
    } catch (e) {
      console.error("启动失败", e);
      const msg = e?.message || String(e);
      const code = e?.code;
      if (code === "NodeMissing") {
        showBanner("未检测到系统 Node.js，请先安装：", "https://nodejs.org/", { retry: refreshEnv });
      } else if (code === "DshMissing") {
        showBanner("未检测到全局 dsh 命令，请安装：", "npm install -g @deepseek/dsh", { retry: refreshEnv });
      } else {
        showBanner("服务启动失败：" + msg, undefined, { retry: startService });
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

  function toggleService() {
    if (state.portInUse && state.owned) stopService();
    else startService();
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

  els.btnToggle.addEventListener("click", toggleService);
  els.btnFullscreen.addEventListener("click", toggleFullscreen);
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
