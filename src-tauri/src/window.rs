//! 窗口与注入脚本：Mica 背景、F11/Esc 全屏快捷键。

/// 全屏快捷键（F11 切换 / Esc 退出）：通过 initialization_script 注入，
/// 每次页面加载都会执行（含整窗导航后的 Harness 远程页），解决主屏全屏时托盘不可达的问题。
/// Esc 仅在事件未被页面自身处理（未 preventDefault / 未 stopPropagation）
/// 且焦点不在输入控件内时生效，优先让 Harness 界面使用 Esc。
/// 走核心窗口命令（getCurrentWindow().setFullscreen）；远程页（Harness）通过
/// capabilities/harness-remote.json 获得 core:window:allow-*-fullscreen 权限。
/// WebView2 无浏览器自带 F11/Esc 全屏行为，必须由脚本自行处理。
pub const SHORTCUT_SCRIPT: &str = r#"(() => {
  if (window.__dshShortcutsInstalled) return;
  window.__dshShortcutsInstalled = true;
  const getWindow = () => {
    const T = window.__TAURI__;
    return T && T.window && T.window.getCurrentWindow ? T.window.getCurrentWindow() : null;
  };
  const isEditable = (el) => {
    if (!el) return false;
    const tag = el.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable === true;
  };
  window.addEventListener('keydown', (e) => {
    const win = getWindow();
    if (!win) return;
    if (e.key === 'F11') {
      e.preventDefault();
      win.isFullscreen().then((fs) => win.setFullscreen(!fs)).catch(() => {});
      return;
    }
    if (e.key === 'Escape' && !e.defaultPrevented && !isEditable(e.target)) {
      win.isFullscreen().then((fs) => { if (fs) win.setFullscreen(false); }).catch(() => {});
    }
  });
})();"#;

/// 应用 Windows 11 Mica 材质（失败时降级为纯色，不致命）。
#[cfg(target_os = "windows")]
pub fn apply_window_effects(window: &tauri::WebviewWindow, dark: bool) {
    match window_vibrancy::apply_mica(window, Some(dark)) {
        Ok(()) => {}
        Err(e) => eprintln!("[dsh-desktop] Mica 背景应用失败（非致命）: {e}"),
    }
}
