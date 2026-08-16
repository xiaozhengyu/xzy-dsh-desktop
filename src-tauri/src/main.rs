// DSH 桌面端 —— Tauri 2 外壳（仅 Windows 11）
//
// 设计要点：
//  - 不内嵌任何 Node.js 运行时：启动时直接调用系统 PATH 中的全局 node 与 dsh。
//  - 通过解析 npm 生成的 dsh.cmd shim 得到真实 JS 入口，用 Command::new(node) 直接执行，
//    从而满足“Rust 后端通过 node 执行 dsh web --port 3081”的要求；解析失败时回退到
//    `cmd /C dsh web --port 3081`。
//  - 进程树清理使用 Windows 原生 taskkill /T /F（比 sysinfo 更轻、更可靠）。
//  - 关闭主窗口 → 最小化到托盘；托盘“退出应用” → 先杀掉派生进程再退出。
//
// 模块划分：
//  - config.rs      配置模型 / 读写 / 注册表自启 / 主题解析 / 设置命令
//  - env.rs         环境检测（node/dsh 定位、shim 解析）
//  - service.rs     服务生命周期（端口/进程探测、启停/重启、状态）
//  - logging.rs     dsh-web.log 增量读取 / 清空 / 定位 / 退出标记
//  - diagnostics.rs 自检诊断
//  - stale.rs       异常退出 / 外部占用检测与身份校验清理
//  - tray.rs        系统托盘
//  - window.rs      Mica 背景与 F11/Esc 快捷键注入
//  - state.rs       全局共享状态
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod diagnostics;
mod env;
mod logging;
mod service;
mod stale;
mod state;
mod tray;
mod window;

use std::sync::{atomic::AtomicBool, Mutex};

use tauri::{Manager, RunEvent, WindowEvent};

use crate::config::AppConfig;
use crate::service::kill_tree;
use crate::state::AppState;
use crate::window::SHORTCUT_SCRIPT;

fn main() {
    let cfg = AppConfig::load();
    let setup_cfg = cfg.clone();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 第二个实例启动时，唤醒已有窗口
            crate::tray::show_main(app);
        }))
        .manage(AppState {
            child: Mutex::new(None),
            exiting: AtomicBool::new(false),
            env_info: Mutex::new(None),
            config: Mutex::new(cfg.clone()),
            started_at: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            crate::env::get_env_info,
            crate::config::get_config,
            crate::service::get_status,
            crate::service::get_service_info,
            crate::logging::open_log_folder,
            crate::logging::tail_log,
            crate::logging::clear_log,
            crate::service::start_service,
            crate::service::stop_service,
            crate::service::restart_service,
            crate::diagnostics::run_diagnostics,
            crate::config::set_config,
            crate::config::set_autostart,
            crate::config::apply_theme,
            crate::config::get_config_path,
            crate::stale::check_stale_info,
            crate::stale::clean_stale
        ])
        .setup(move |app| {
            // 主窗口在 Rust 侧创建：需要挂载 initialization_script，
            // 向每次页面加载（含整窗导航后的 Harness 页）注入全屏快捷键（F11/Esc）。
            let win = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("DSH 桌面端")
            .inner_size(1280.0, 840.0)
            .min_inner_size(940.0, 620.0)
            .center()
            .resizable(true)
            .decorations(true)
            .visible(true)
            .additional_browser_args("--proxy-bypass-list=<-loopback>")
            .devtools(true)
            .initialization_script(SHORTCUT_SCRIPT)
            .build()?;

            #[cfg(target_os = "windows")]
            crate::window::apply_window_effects(
                &win,
                crate::config::resolve_theme_dark(&setup_cfg.theme.mode),
            );

            // 按配置决定是否自动打开 DevTools（调试用）
            #[cfg(any(debug_assertions, feature = "devtools"))]
            if setup_cfg.devtools.auto_open {
                let _ = win.open_devtools();
            }

            crate::tray::build_tray(app.handle())?;
            crate::tray::sync_fs_menu_label(app.handle());

            // 自动启动服务（配置 service.autoStart，默认开）
            if setup_cfg.service.auto_start {
                let app = app.handle().clone();
                std::thread::spawn(move || {
                    let st = app.state::<AppState>();
                    let cfg = st.config.lock().unwrap().clone();
                    if let Err(e) = crate::service::start_service_inner(&app, &st, &cfg) {
                        eprintln!("[dsh-desktop] 自动启动服务失败: {e:?}");
                    }
                });
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // 主窗口关闭 → 最小化到托盘（而非退出）
            if window.label() == "main" {
                match event {
                    WindowEvent::CloseRequested { api, .. } => {
                        if !window.app_handle().state::<AppState>().exiting.load(std::sync::atomic::Ordering::SeqCst) {
                            api.prevent_close();
                            let _ = window.hide();
                        }
                    }
                    // 进入/退出全屏会触发窗口尺寸变化：借此同步托盘菜单文案
                    WindowEvent::Resized(_) => crate::tray::sync_fs_menu_label(window.app_handle()),
                    _ => {}
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("构建 Tauri 应用失败");

    app.run(|app_handle, event| {
        // 退出时写正常退出标记并强杀由本应用派生的 dsh 进程树，避免端口占用残留。
        // 异常强杀（任务管理器）不会经过这里 → 日志尾部无标记，前端据此提示上次异常退出。
        if let RunEvent::Exit = event {
            crate::logging::append_log_line(app_handle, "[dsh-desktop] --- normal shutdown ---");
            if let Some(mut child) = app_handle.state::<AppState>().child.lock().unwrap().take() {
                kill_tree(child.id());
                let _ = child.wait();
            }
        }
    });
}
