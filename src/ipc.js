// ipc.js —— Tauri invoke 统一封装（在 main.js 之前加载）
(() => {
  "use strict";
  const TAURI = window.__TAURI__;
  const invoke = TAURI?.core?.invoke;

  window.DSH = window.DSH || {};
  window.DSH.inTauri = !!invoke;
  window.DSH.invoke = (cmd, args) => {
    if (!invoke) return Promise.reject(new Error("not-in-tauri"));
    return invoke(cmd, args);
  };
  // 应用版本（关于卡片展示；失败回退 0.1.0）
  window.DSH.getAppVersion = async () => {
    try {
      const app = window.__TAURI__?.app;
      if (app && app.getVersion) return await app.getVersion();
    } catch (e) { /* 忽略 */ }
    return "0.1.0";
  };
})();
