//! 系统托盘：菜单构建、窗口唤起、全屏切换与菜单文案同步。

use std::sync::OnceLock;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager,
};

use crate::state::AppState;

/// 托盘「进入全屏 / 退出全屏」菜单项句柄（文案随主窗口全屏状态切换）。
static FS_MENU_ITEM: OnceLock<MenuItem<tauri::Wry>> = OnceLock::new();

/// 唤起主窗口到前台。
pub fn show_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// 根据主窗口当前全屏状态，同步托盘全屏菜单文案。
pub fn sync_fs_menu_label(app: &AppHandle) {
    let Some(item) = FS_MENU_ITEM.get() else { return };
    let is_fs = app
        .get_webview_window("main")
        .and_then(|win| win.is_fullscreen().ok())
        .unwrap_or(false);
    let _ = item.set_text(if is_fs { "退出全屏" } else { "进入全屏" });
}

/// 切换主窗口全屏状态，并同步托盘菜单文案（托盘菜单与快捷键共用）。
fn toggle_fullscreen_app(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let is_fs = win.is_fullscreen().unwrap_or(false);
        let _ = win.set_fullscreen(!is_fs);
        sync_fs_menu_label(app);
    }
}

/// 构建托盘（菜单自上而下：进入全屏 / 退出全屏、控制面板、功能设置、退出应用）。
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let fs = MenuItem::with_id(app, "fullscreen", "进入全屏", true, None::<&str>)?;
    let back = MenuItem::with_id(app, "back", "控制面板", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "功能设置", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出应用", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&fs, &back, &settings, &quit])?;
    let _ = FS_MENU_ITEM.set(fs.clone());

    // 用 32×32 PNG 作为托盘图标，DPI 下更清晰
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .expect("无法加载托盘图标");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("DSH 桌面端")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "back" => {
                // 从 Harness 界面回到应用控制页（Windows 生产环境的应用页面地址）
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.navigate(tauri::Url::parse("http://tauri.localhost/#/").expect("应用页 URL 解析失败"));
                }
            }
            "settings" => {
                // 从 Harness 界面直达设置视图
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.unminimize();
                    let _ = win.navigate(tauri::Url::parse("http://tauri.localhost/#/settings").expect("应用页 URL 解析失败"));
                }
            }
            "fullscreen" => toggle_fullscreen_app(app),
            "quit" => {
                app.state::<AppState>().exiting.store(true, std::sync::atomic::Ordering::SeqCst);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
