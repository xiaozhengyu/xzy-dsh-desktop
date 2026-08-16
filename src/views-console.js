// views-console.js —— 控制台视图：服务/环境卡片、操作行、日志查看器
(() => {
  "use strict";
  const DSH = window.DSH;
  const invoke = DSH.invoke;
  const state = DSH.state;
  const $ = (id) => document.getElementById(id);

  const els = {
    svcStatus: $("svc-status"),
    svcUrl: $("svc-url"),
    svcPid: $("svc-pid"),
    svcUptime: $("svc-uptime"),
    envNode: $("env-node"),
    envDsh: $("env-dsh"),
    btnEnter: $("btn-enter-harness"),
    btnRestart: $("btn-restart"),
    btnDiag: $("btn-diagnostics"),
    btnUpdate: $("btn-update"),
    updateResult: $("update-result"),
    logView: $("log-view"),
    logFilter: $("log-filter"),
    logToBottom: $("log-scroll-bottom"),
    diagPanel: $("diag-panel"),
    diagList: $("diag-list"),
  };

  // ---------------- 日志查看器状态 ----------------
  const MAX_LINES = 2000;
  const rawLines = [];          // 全部行（原始，带分类）
  const logState = { offset: 0, follow: true, filter: "", timer: null };

  function isConsoleVisible() {
    return !DSH.els.viewConsole.classList.contains("hidden");
  }

  // ---------------- 服务卡片 ----------------
  function fmtUptime(ms) {
    if (!ms || ms < 0) return "—";
    const s = Math.floor(ms / 1000);
    const d = Math.floor(s / 86400);
    const h = Math.floor((s % 86400) / 3600);
    const m = Math.floor((s % 3600) / 60);
    if (d > 0) return `${d} 天 ${h} 小时`;
    if (h > 0) return `${h} 小时 ${m} 分`;
    return `${m} 分 ${s % 60} 秒`;
  }

  function renderServiceCard() {
    const info = DSH.serviceInfo || {};
    els.svcStatus.textContent = state.portInUse
      ? state.owned ? "运行中（本应用托管）" : "运行中（外部实例）"
      : "已停止";
    els.svcUrl.textContent = state.webUrl || "—";
    els.svcPid.textContent = info.pid ? String(info.pid) : "—";
    els.svcUptime.textContent = state.owned && info.startedAtMs
      ? fmtUptime(Date.now() - info.startedAtMs)
      : "—";
  }

  async function refreshServiceInfo() {
    if (!DSH.inTauri) return;
    try {
      DSH.serviceInfo = await invoke("get_service_info");
    } catch (e) {
      console.error("get_service_info 失败", e);
    }
  }

  // ---------------- 环境卡片 ----------------
  function renderEnv() {
    const env = state.env;
    if (!env) {
      els.envNode.textContent = "检测中…";
      els.envDsh.textContent = "检测中…";
      return;
    }
    els.envNode.textContent = env.node
      ? `找到（${env.node}）`
      : "未找到 — 请安装 Node.js";
    els.envDsh.textContent = env.dsh
      ? `找到（${env.dsh}）`
      : "未找到 — 请执行 npm install -g @deepseek-ai/dsh";
  }

  // ---------------- 操作行 ----------------
  function renderActions() {
    els.btnEnter.classList.toggle("hidden", !state.portInUse);
    els.btnRestart.classList.toggle("hidden", !(state.portInUse && state.owned));
  }

  function flash(btn, text, ms = 1200) {
    const old = btn.textContent;
    btn.textContent = text;
    btn.disabled = true;
    setTimeout(() => {
      btn.textContent = old;
      btn.disabled = false;
    }, ms);
  }

  async function copyText(text, btn) {
    try {
      await navigator.clipboard.writeText(text);
      flash(btn, "已复制");
    } catch {
      try {
        const ta = document.createElement("textarea");
        ta.value = text;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        ta.remove();
        flash(btn, "已复制");
      } catch (e) {
        console.error("复制失败", e);
      }
    }
  }

  // ---------------- 日志查看器 ----------------
  function classify(line) {
    if (/error|fail|panic|exception|异常|失败|traceback/i.test(line)) return "error";
    if (/boot|ready|listen|start|启动|就绪|web\b/i.test(line)) return "info";
    return "";
  }

  function matchesFilter(text) {
    const f = logState.filter.toLowerCase();
    return !f || text.toLowerCase().includes(f);
  }

  function pushLine(text, cls) {
    rawLines.push({ text, cls });
    if (rawLines.length > MAX_LINES) rawLines.splice(0, rawLines.length - MAX_LINES);
  }

  function isAtBottom() {
    const el = els.logView;
    return el.scrollHeight - el.scrollTop - el.clientHeight < 30;
  }

  function scrollToBottom() {
    els.logView.scrollTop = els.logView.scrollHeight;
  }

  function rebuildLog() {
    els.logView.textContent = "";
    const frag = document.createDocumentFragment();
    for (const { text, cls } of rawLines) {
      if (!matchesFilter(text)) continue;
      const div = document.createElement("div");
      div.className = "log-line" + (cls ? " " + cls : "");
      div.textContent = text;
      frag.appendChild(div);
    }
    els.logView.appendChild(frag);
    if (logState.follow) scrollToBottom();
  }

  function appendLines(lines) {
    const atBottom = isAtBottom();
    const frag = document.createDocumentFragment();
    for (const text of lines) {
      const cls = classify(text);
      pushLine(text, cls);
      if (!matchesFilter(text)) continue;
      const div = document.createElement("div");
      div.className = "log-line" + (cls ? " " + cls : "");
      div.textContent = text;
      frag.appendChild(div);
    }
    els.logView.appendChild(frag);
    if (logState.follow && atBottom) scrollToBottom();
  }

  async function pollLog() {
    if (!DSH.inTauri || document.hidden || !isConsoleVisible()) return;
    try {
      const res = await invoke("tail_log", { offset: logState.offset });
      if (res.truncated) {
        logState.offset = res.offset;
        rawLines.length = 0;
        els.logView.textContent = "";
        pushLine("--- 日志已被清空 / 重置 ---", "info");
        rebuildLog();
        return;
      }
      logState.offset = res.offset;
      if (res.lines.length) appendLines(res.lines);
    } catch (e) {
      /* 读取失败静默（文件可能被占用），下轮重试 */
    }
  }

  function startLogTimer() {
    if (logState.timer || !DSH.inTauri) return;
    refreshServiceInfo();
    pollLog();
    logState.timer = setInterval(() => {
      refreshServiceInfo();
      pollLog();
    }, 1500);
  }

  // ---------------- 操作行：重启 / 自检 / 检查更新（P1） ----------------

  DSH.restartService = async () => {
    if (!DSH.inTauri || state.busy) return;
    const btn = els.btnRestart;
    btn.disabled = true;
    btn.textContent = "重启中…";
    try {
      await invoke("restart_service");
      await DSH.refreshStatus();
    } catch (e) {
      console.error("重启失败", e);
      DSH.showBanner("重启失败：" + (e?.message || String(e)), undefined, {
        retry: DSH.restartService,
      });
    } finally {
      btn.disabled = false;
      btn.textContent = "重启服务";
      await DSH.refreshStatus();
    }
  };

  DSH.runDiagnostics = async () => {
    if (!DSH.inTauri) return;
    els.diagPanel.classList.remove("hidden");
    els.diagList.textContent = "";
    const loading = document.createElement("li");
    loading.textContent = "正在诊断…";
    els.diagList.appendChild(loading);
    try {
      const items = await invoke("run_diagnostics");
      els.diagList.textContent = "";
      DSH.lastDiag = items;
      for (const it of items) {
        const li = document.createElement("li");
        const mark = document.createElement("span");
        mark.className = it.ok ? "diag-ok" : "diag-bad";
        mark.textContent = it.ok ? "✓" : "✗";
        const detail = document.createElement("span");
        detail.className = "diag-item-detail";
        detail.textContent = `${it.check}：${it.detail}`;
        li.appendChild(mark);
        li.appendChild(detail);
        els.diagList.appendChild(li);
      }
    } catch (e) {
      els.diagList.textContent = "";
      const li = document.createElement("li");
      li.textContent = "诊断执行失败：" + (e?.message || String(e));
      els.diagList.appendChild(li);
    }
  };

  DSH.checkUpdate = async () => {
    if (!DSH.inTauri) return;
    els.updateResult.textContent = "检查中…";
    els.updateResult.className = "action-hint";
    try {
      const info = await invoke("check_update");
      if (info.hasUpdate) {
        els.updateResult.textContent = `发现新版本：${info.current} → ${info.latest}`;
        els.updateResult.className = "action-hint warn";
      } else {
        els.updateResult.textContent = `已是最新（${info.current}）`;
        els.updateResult.className = "action-hint ok";
      }
    } catch (e) {
      console.error("检查更新失败", e);
      els.updateResult.textContent = e?.message || "检查失败";
      els.updateResult.className = "action-hint";
    }
  };

  els.btnRestart.addEventListener("click", () => DSH.restartService());
  els.btnDiag.addEventListener("click", () => DSH.runDiagnostics());
  els.btnUpdate.addEventListener("click", () => DSH.checkUpdate());
  $("btn-copy-diag").addEventListener("click", () => {
    if (!DSH.lastDiag) return;
    const text = DSH.lastDiag
      .map((it) => `${it.ok ? "[OK]" : "[FAIL]"} ${it.check}: ${it.detail}`)
      .join("\n");
    copyText(text, $("btn-copy-diag"));
  });
  $("btn-close-diag").addEventListener("click", () => {
    els.diagPanel.classList.add("hidden");
  });

  // ---------------- P3：异常退出检测与残留清理 ----------------
  let abnormalChecked = false;

  async function checkAbnormalExit() {
    if (!DSH.inTauri || abnormalChecked) return;
    abnormalChecked = true;
    try {
      const abnormal = await invoke("check_abnormal_exit");
      if (!abnormal) return;
      DSH.showBanner("检测到上次可能异常退出，dsh 进程可能残留", undefined, {
        info: true,
        action: {
          label: "一键清理残留进程",
          fn: async () => {
            try {
              const res = await invoke("clean_stale");
              if (res.cleaned) {
                DSH.hideBanner();
              } else {
                DSH.showBanner(res.detail, undefined, { info: true });
              }
              await DSH.refreshStatus();
            } catch (e) {
              DSH.showBanner("清理失败：" + (e?.message || String(e)));
            }
          },
        },
      });
    } catch (e) {
      /* 检测失败静默 */
    }
  }

  // ---------------- 事件绑定（P0） ----------------
  els.btnEnter.addEventListener("click", DSH.navigateToHarness);
  $("btn-open-log").addEventListener("click", () => {
    invoke("open_log_folder").catch((e) => console.error("打开日志文件夹失败", e));
  });
  $("btn-recheck-env").addEventListener("click", () => DSH.refreshEnv(true));
  $("btn-clear-log").addEventListener("click", async () => {
    if (!confirm("确定清空日志文件吗？")) return;
    try {
      await invoke("clear_log");
      logState.offset = 0;
      rawLines.length = 0;
      els.logView.textContent = "";
      pushLine("--- 日志已清空 ---", "info");
      rebuildLog();
    } catch (e) {
      console.error("清空日志失败", e);
    }
  });
  els.logFilter.addEventListener("input", () => {
    logState.filter = els.logFilter.value.trim();
    rebuildLog();
  });
  els.logView.addEventListener("scroll", () => {
    logState.follow = isAtBottom();
    els.logToBottom.classList.toggle("hidden", logState.follow);
  });
  els.logToBottom.addEventListener("click", () => {
    logState.follow = true;
    scrollToBottom();
    els.logToBottom.classList.add("hidden");
  });

  // ---------------- 对外接口 ----------------
  DSH.renderConsole = () => {
    renderServiceCard();
    renderEnv();
    renderActions();
  };

  DSH.onConsoleVisible = () => {
    startLogTimer();
    checkAbnormalExit();
  };
})();
