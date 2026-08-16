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
})();
