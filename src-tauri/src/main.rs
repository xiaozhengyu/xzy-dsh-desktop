// DeepSeek Harness 桌面端 —— Tauri 2 外壳（仅 Windows 11）
//
// 设计要点：
//  - 不内嵌任何 Node.js 运行时：启动时直接调用系统 PATH 中的全局 node 与 dsh。
//  - 通过解析 npm 生成的 dsh.cmd shim 得到真实 JS 入口，用 Command::new(node) 直接执行，
//    从而满足“Rust 后端通过 node 执行 dsh web --port 3080”的要求；解析失败时回退到
//    `cmd /C dsh web --port 3080`。
//  - 进程树清理使用 Windows 原生 taskkill /T /F（比 sysinfo 更轻、更可靠）。
//  - 关闭主窗口 → 最小化到托盘；托盘“退出应用” → 先杀掉派生进程再退出。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    fs::{self, File},
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, RunEvent, State, WindowEvent,
};

const WEB_PORT: u16 = 3080;
const WEB_URL: &str = "http://127.0.0.1:3080";
const START_TIMEOUT: Duration = Duration::from_secs(25);

struct AppState {
    child: Mutex<Option<Child>>,
    exiting: AtomicBool,
}

// ---------------------------------------------------------------- 序列化类型

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnvInfo {
    node: Option<String>,
    dsh: Option<String>,
    dsh_entry: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusInfo {
    port_in_use: bool,
    owned: bool,
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
enum StartOutcome {
    Started { url: String },
    AlreadyRunning { url: String },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "code", content = "message")]
enum StartError {
    NodeMissing,
    DshMissing,
    SpawnFailed(String),
    NotReady(String),
}

// ---------------------------------------------------------------- 系统工具

/// 在 PATH 中查找可执行文件，返回第一个带扩展名（.cmd/.exe）的条目。
fn where_find(exe: &str) -> Option<String> {
    let out = Command::new("where").arg(exe).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    lines
        .iter()
        .find(|l| l.to_lowercase().ends_with(".cmd") || l.to_lowercase().ends_with(".exe"))
        .cloned()
        .or_else(|| lines.first().cloned())
}

/// 解析 npm 生成的 .cmd shim（如 dsh.cmd），取出真正的 JS 入口。
/// 典型内容：`"%_prog%"  "%dp0%\node_modules\@deepseek-ai\dsh\lib\bin.js" %*`
fn resolve_dsh_entry(shim: &str) -> Option<String> {
    let content = fs::read_to_string(shim).ok()?;
    let idx = content.find("node_modules")?;
    let head = &content[..idx];
    let start = head.rfind('"').or_else(|| head.rfind('%'))?;
    let tail = &content[idx..];
    let end = tail.find('"').map(|i| idx + i).unwrap_or(content.len());
    let raw = content[start..end].to_string();
    let shim_dir = std::path::Path::new(shim).parent()?.to_string_lossy().into_owned();
    let resolved = raw
        .replace("%~dp0", &shim_dir)
        .replace("%dp0", &shim_dir)
        .trim_matches('"')
        .to_string();
    let path = PathBuf::from(resolved);
    if path.exists() {
        Some(path.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// 检测端口是否被占用（127.0.0.1 / ::1）。
fn port_in_use(port: u16) -> bool {
    for host in ["127.0.0.1", "::1"] {
        let text = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let Ok(mut addrs) = text.to_socket_addrs() else {
            continue;
        };
        if addrs.any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(250)).is_ok()) {
            return true;
        }
    }
    false
}

/// 用 Windows 原生 taskkill 强杀进程树（含子进程），防止端口残留。
fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .arg("/PID")
        .arg(pid.to_string())
        .arg("/T")
        .arg("/F")
        .output();
}

// ---------------------------------------------------------------- 窗口 / 托盘

fn show_main(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

#[cfg(target_os = "windows")]
fn apply_window_effects(window: &tauri::WebviewWindow) {
    match window_vibrancy::apply_mica(window, Some(true)) {
        Ok(()) => {}
        Err(e) => eprintln!("[dsh-desktop] Mica 背景应用失败（非致命）: {e}"),
    }
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出应用", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    // 用 32×32 PNG 作为托盘图标，DPI 下更清晰
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
        .expect("无法加载托盘图标");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("DeepSeek Harness 桌面端")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "quit" => {
                app.state::<AppState>().exiting.store(true, Ordering::SeqCst);
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

// ---------------------------------------------------------------- 命令

#[tauri::command]
fn get_env_info() -> EnvInfo {
    let node = where_find("node");
    let dsh = where_find("dsh");
    let dsh_entry = dsh.as_deref().and_then(resolve_dsh_entry);
    EnvInfo {
        node,
        dsh,
        dsh_entry,
    }
}

#[tauri::command]
fn get_status(state: State<'_, AppState>) -> StatusInfo {
    let mut owned = false;
    {
        let mut guard = state.child.lock().unwrap();
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // 进程已退出，清理句柄
                    *guard = None;
                }
                _ => owned = true,
            }
        }
    }
    StatusInfo {
        port_in_use: port_in_use(WEB_PORT),
        owned,
        url: WEB_URL.into(),
    }
}

#[tauri::command]
fn start_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<StartOutcome, StartError> {
    // 1) 环境检测：node 与全局 dsh 必须存在
    let node = where_find("node").ok_or(StartError::NodeMissing)?;
    let dsh_shim = where_find("dsh").ok_or(StartError::DshMissing)?;

    // 2) 端口占用检测：已被占用 → 视为“Harness 可能已在运行”，不重复启动
    if port_in_use(WEB_PORT) {
        return Ok(StartOutcome::AlreadyRunning {
            url: WEB_URL.into(),
        });
    }

    // 清理可能残留的旧进程
    if let Some(mut old) = state.child.lock().unwrap().take() {
        kill_tree(old.id());
        let _ = old.wait();
    }

    // 3) 构造命令：优先 node <dsh 真实入口>，解析失败回退 cmd /C dsh
    let mut cmd = match resolve_dsh_entry(&dsh_shim) {
        Some(entry) => {
            let mut c = Command::new(&node);
            c.arg(&entry);
            c
        }
        None => {
            let mut c = Command::new("cmd");
            c.args(["/C", "dsh"]);
            c
        }
    };
    cmd.arg("web").arg("--port").arg(WEB_PORT.to_string());

    // 日志落盘到应用数据目录，便于排错
    let log_dir = app
        .path()
        .app_log_dir()
        .map_err(|e| StartError::SpawnFailed(format!("无法获取日志目录: {e}")))?;
    let _ = fs::create_dir_all(&log_dir);
    let log_file = log_dir.join("dsh-web.log");
    let stdout_file = File::options()
        .create(true)
        .append(true)
        .open(&log_file)
        .map_err(|e| StartError::SpawnFailed(format!("无法打开日志文件: {e}")))?;
    let stderr_file = stdout_file
        .try_clone()
        .map_err(|e| StartError::SpawnFailed(format!("无法克隆日志句柄: {e}")))?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));

    let child = cmd
        .spawn()
        .map_err(|e| StartError::SpawnFailed(format!("启动 dsh 失败: {e}")))?;
    *state.child.lock().unwrap() = Some(child);

    // 4) 等待 3080 端口就绪
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if port_in_use(WEB_PORT) {
            return Ok(StartOutcome::Started {
                url: WEB_URL.into(),
            });
        }
        {
            let mut guard = state.child.lock().unwrap();
            if let Some(child) = guard.as_mut() {
                if let Ok(Some(_)) = child.try_wait() {
                    // 进程已退出：清空句柄，避免后续 get_status 把死进程误判为 owned
                    *guard = None;
                    return Err(StartError::NotReady(format!(
                        "dsh 进程提前退出，请查看日志: {}",
                        log_file.display()
                    )));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(StartError::NotReady(format!(
        "启动超时（{}s），请查看日志: {}",
        START_TIMEOUT.as_secs(),
        log_file.display()
    )))
}

#[tauri::command]
fn stop_service(state: State<'_, AppState>) -> Result<(), String> {
    let child = state.child.lock().unwrap().take();
    if let Some(mut c) = child {
        kill_tree(c.id());
        let _ = c.wait();
    }
    Ok(())
}

// ---------------------------------------------------------------- 入口

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 第二个实例启动时，唤醒已有窗口
            show_main(app);
        }))
        .manage(AppState {
            child: Mutex::new(None),
            exiting: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![
            get_env_info,
            get_status,
            start_service,
            stop_service
        ])
        .setup(|app| {
            build_tray(app.handle())?;
            #[cfg(target_os = "windows")]
            if let Some(win) = app.get_webview_window("main") {
                apply_window_effects(&win);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 主窗口关闭 → 最小化到托盘（而非退出）
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if !window.app_handle().state::<AppState>().exiting.load(Ordering::SeqCst) {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("构建 Tauri 应用失败");

    app.run(|app_handle, event| {
        // 退出时强杀由本应用派生的 dsh 进程树，避免端口占用残留
        if let RunEvent::Exit = event {
            if let Some(mut child) = app_handle.state::<AppState>().child.lock().unwrap().take() {
                kill_tree(child.id());
                let _ = child.wait();
            }
        }
    });
}
