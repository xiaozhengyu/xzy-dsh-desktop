// main.js —— 入口：共享状态、hash 路由、状态轮询、启停流程、环境检测
// 通过 window.DSH 命名空间与 views-console.js / views-settings.js 协作。
(() => {
  "use strict";
  const DSH = window.DSH;
  const invoke = DSH.invoke;
  const $ = (id) => document.getElementById(id);

  // ---------------- 共享状态 ----------------
  DSH.state = {
    env: null,
    portInUse: false,
    owned: false,
    busy: false,
    busyAction: "启动",
    webPort: 3081,
    webUrl: "http://127.0.0.1:3081",
    config: null,
    runningAtLoad: null, // 首次刷新时服务是否已在运行（托盘返回时为 true，不自动跳转）
    navigated: false,
  };
  const state = DSH.state;

  // ---------------- 元素引用 ----------------
  DSH.els = {
    banner: $("banner"),
    bannerText: $("banner-text"),
    bannerCmd: $("banner-cmd"),
    btnRetry: $("btn-retry"),
    viewConsole: $("view-console"),
    viewSettings: $("view-settings"),
    statusDot: $("status-dot"),
    statusText: $("status-text"),
    btnToggle: $("btn-toggle"),
  };
  const els = DSH.els;

  // ---------------- 横幅 ----------------
  DSH.showBanner = (text, cmdText, opts = {}) => {
    els.banner.className = "banner" + (opts.info ? " info" : "");
    els.bannerText.textContent = text;
    els.bannerCmd.textContent = cmdText || "";
    els.bannerCmd.classList.toggle("hidden", !cmdText);
    els.btnRetry.classList.toggle("hidden", !opts.retry);
    els.btnRetry.onclick = opts.retry || null;
  };
  DSH.hideBanner = () => els.banner.classList.add("hidden");

  // ---------------- 状态渲染 ----------------
  function setStatus(kind, text, detail) {
    els.statusDot.className = "status-dot " + kind;
    els.statusText.textContent = text;
    els.statusText.title = detail || "";
  }

  function statusDetail() {
    const { portInUse, owned, busy, busyAction, webPort, webUrl } = state;
    if (busy) return busyAction === "停止" ? "正在终止 dsh 进程树…" : `等待 ${webPort} 端口就绪…`;
    if (portInUse && owned) return `dsh web 服务由本应用托管（${webUrl}）`;
    if (portInUse) return `检测到 ${webUrl} 已被占用，未重复启动，已直接加载现有实例`;
    return `点击「启动服务」运行 dsh web --port ${webPort}（${webUrl}）`;
  }

  DSH.render = () => {
    const { portInUse, owned, busy, busyAction, env } = state;

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
      els.btnToggle.disabled = !DSH.inTauri || busy || !owned;
    } else {
      els.btnToggle.textContent = "启动服务";
      els.btnToggle.className = "btn-toggle start";
      els.btnToggle.disabled = !DSH.inTauri || busy || !envOk;
    }

    if (typeof DSH.renderConsole === "function") DSH.renderConsole();
  };

  // ---------------- 导航 ----------------
  function navigateToHarness() {
    window.location.href = state.webUrl;
  }
  DSH.navigateToHarness = navigateToHarness;

  /**
   * 就绪后进入 Harness：端口能连上 ≠ dsh web 后端服务已初始化，
   * 立即跳转可能触发 "web boot: entries did not activate" 竞态，缓冲一段时间再进入。
   */
  async function enterHarnessAfterReady() {
    await new Promise((r) => setTimeout(r, 2500));
    navigateToHarness();
  }

  // ---------------- 环境检测 ----------------
  DSH.refreshEnv = async (force) => {
    if (!DSH.inTauri) {
      DSH.showBanner("当前不在 Tauri 桌面环境中运行", "请使用 npm run tauri dev 启动应用");
      DSH.render();
      return;
    }
    try {
      const env = await invoke("get_env_info", { force: !!force });
      state.env = env;
      if (!env.node || !env.dsh) {
        const missing = [];
        if (!env.node) missing.push("Node.js");
        if (!env.dsh) missing.push("全局 dsh");
        DSH.showBanner(
          `缺少运行环境：${missing.join("、")}`,
          "npm install -g @deepseek/dsh",
          { retry: () => DSH.refreshEnv(true) }
        );
      } else {
        DSH.hideBanner();
      }
      DSH.render();
    } catch (e) {
      console.error("get_env_info 失败", e);
    }
  };

  // ---------------- 数据刷新 ----------------
  async function refreshStatus() {
    if (!DSH.inTauri) return;
    try {
      const s = await invoke("get_status");
      state.portInUse = s.portInUse;
      state.owned = s.owned;
    } catch (e) {
      console.error("get_status 失败", e);
    }
    // 首次刷新记录“服务是否已在运行”：托盘返回时已在运行则不自动跳转
    if (state.runningAtLoad === null) state.runningAtLoad = state.portInUse;
    // 新启动的服务就绪后自动整窗进入 Harness（带就绪缓冲，避免后端未就绪的 boot 竞态）
    if (state.portInUse && !state.runningAtLoad && !state.navigated) {
      state.navigated = true;
      enterHarnessAfterReady();
      return;
    }
    DSH.render();
  }
  DSH.refreshStatus = refreshStatus;

  function startPolling() {
    setInterval(() => {
      if (!document.hidden) refreshStatus();
    }, 3000);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) refreshStatus();
    });
  }

  // ---------------- 启停流程 ----------------
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  async function waitPortUp(timeoutMs) {
    const t0 = Date.now();
    while (Date.now() - t0 < timeoutMs) {
      await refreshStatus();
      if (state.portInUse) return true;
      await sleep(600);
    }
    return state.portInUse;
  }

  async function waitPortDown(timeoutMs) {
    const t0 = Date.now();
    while (Date.now() - t0 < timeoutMs) {
      await refreshStatus();
      if (!state.portInUse) return true;
      await sleep(400);
    }
    return !state.portInUse;
  }

  DSH.startService = async () => {
    if (!DSH.inTauri || state.busy) return;
    DSH.hideBanner();
    state.busy = true;
    state.busyAction = "启动";
    DSH.render();
    try {
      const res = await invoke("start_service");
      if (res && res.status === "alreadyRunning") {
        await refreshStatus();
      } else {
        await waitPortUp(30000);
      }
      if (state.portInUse) enterHarnessAfterReady();
    } catch (e) {
      console.error("启动失败", e);
      const msg = e?.message || String(e);
      const code = e?.code;
      if (code === "NodeMissing") {
        DSH.showBanner("未检测到系统 Node.js，请先安装：", "https://nodejs.org/", { retry: DSH.refreshEnv });
      } else if (code === "DshMissing") {
        DSH.showBanner("未检测到全局 dsh 命令，请安装：", "npm install -g @deepseek/dsh", { retry: DSH.refreshEnv });
      } else {
        DSH.showBanner("服务启动失败：" + msg, undefined, { retry: DSH.startService });
      }
      setStatus("error", "启动失败", msg);
    } finally {
      state.busy = false;
      await refreshStatus();
    }
  };

  DSH.stopService = async () => {
    if (!DSH.inTauri || state.busy) return;
    state.busy = true;
    state.busyAction = "停止";
    DSH.render();
    try {
      await invoke("stop_service");
      await waitPortDown(10000);
    } catch (e) {
      console.error("停止失败", e);
    } finally {
      state.busy = false;
      await refreshStatus();
    }
  };

  function toggleService() {
    if (state.portInUse && state.owned) DSH.stopService();
    else DSH.startService();
  }

  // ---------------- 配置 ----------------
  async function applyConfig() {
    if (!DSH.inTauri) return;
    try {
      const cfg = await invoke("get_config");
      if (cfg && cfg.webPort) {
        state.webPort = cfg.webPort;
        state.webUrl = cfg.webUrl;
      }
      state.config = cfg;
    } catch (e) {
      console.error("get_config 失败，使用默认参数", e);
    }
  }
  // 设置页保存后重新拉取配置（端口/主题等变化同步到共享状态）
  DSH.reloadConfig = async () => {
    await applyConfig();
    DSH.render();
    if (typeof DSH.renderConsole === "function") DSH.renderConsole();
  };

  // ---------------- hash 路由（#/ 控制台 / #/settings 设置） ----------------
  function route() {
    const hash = window.location.hash || "#/";
    const isSettings = hash.startsWith("#/settings");
    els.viewConsole.classList.toggle("hidden", isSettings);
    els.viewSettings.classList.toggle("hidden", !isSettings);
    if (isSettings && typeof DSH.renderSettings === "function") DSH.renderSettings();
    if (!isSettings && typeof DSH.onConsoleVisible === "function") DSH.onConsoleVisible();
  }
  DSH.route = route;

  // ---------------- 初始化 ----------------
  els.btnToggle.addEventListener("click", toggleService);
  $("btn-settings").addEventListener("click", () => { window.location.hash = "#/settings"; });
  $("btn-back-console").addEventListener("click", () => { window.location.hash = "#/"; });
  window.addEventListener("hashchange", route);

  if (!DSH.inTauri) {
    DSH.showBanner("当前不在 Tauri 桌面环境中运行", "请使用 npm run tauri dev 启动应用");
    DSH.render();
    return;
  }

  (async () => {
    await applyConfig(); // 先从 config.json 读取端口等参数
    DSH.refreshEnv();
    refreshStatus();
    startPolling();
    route();
  })();
})();
