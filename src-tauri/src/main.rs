// DSH 桌面端 —— Tauri 2 外壳（仅 Windows 11）
//
// 设计要点：
//  - 不内嵌任何 Node.js 运行时：启动时直接调用系统 PATH 中的全局 node 与 dsh。
//  - 通过解析 npm 生成的 dsh.cmd shim 得到真实 JS 入口，用 Command::new(node) 直接执行，
//    从而满足“Rust 后端通过 node 执行 dsh web --port 3081”的要求；解析失败时回退到
//    `cmd /C dsh web --port 3081`。
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
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, RunEvent, State, WindowEvent,
};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ---------------------------------------------------------------- 配置（config.json）

/// 配置结构体：首次运行在应用配置目录生成 config.json，修改后重启生效。
/// 位置：%APPDATA%\com.deepseek.harness-desktop\config.json
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
struct WebConfig {
    host: String,
    port: u16,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self { host: "127.0.0.1".into(), port: 3081 }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
struct ServiceConfig {
    start_timeout_secs: u64,
    auto_start: bool,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self { start_timeout_secs: 25, auto_start: true }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
struct DevtoolsConfig {
    auto_open: bool,
}

impl Default for DevtoolsConfig {
    fn default() -> Self {
        Self { auto_open: false }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
struct AppConfig {
    note: String,
    web: WebConfig,
    service: ServiceConfig,
    devtools: DevtoolsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            note: "DSH 桌面端配置文件。修改后重启应用生效。web.port 为 dsh web 服务端口（默认 3081，避开 Harness 默认 3080 可与现有会话并存）；web.host 为绑定主机；service.startTimeoutSecs 为服务启动等待上限（秒）；service.autoStart 为启动应用时自动拉起服务；devtools.autoOpen 为调试用自动打开开发者工具。".into(),
            web: WebConfig::default(),
            service: ServiceConfig::default(),
            devtools: DevtoolsConfig::default(),
        }
    }
}

impl AppConfig {
    /// 组装 Web 访问地址。
    fn web_url(&self) -> String {
        format!("http://{}:{}", self.web.host, self.web.port)
    }

    /// 配置文件路径（无需 AppHandle，供窗口创建前读取）。
    fn config_path() -> Option<std::path::PathBuf> {
        std::env::var("APPDATA").ok().map(|a| {
            std::path::PathBuf::from(a)
                .join("com.deepseek.harness-desktop")
                .join("config.json")
        })
    }

    /// 读取配置文件；不存在则写入默认值；解析失败回退默认值。
    fn load() -> Self {
        let def = Self::default();
        let Some(file) = Self::config_path() else {
            return def;
        };
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if !file.exists() {
            if let Ok(json) = serde_json::to_string_pretty(&def) {
                let _ = fs::write(&file, json);
            }
            return def;
        }
        match fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(cfg) => cfg,
            None => {
                eprintln!("[dsh-desktop] 配置文件解析失败，使用默认值: {}", file.display());
                def
            }
        }
    }
}

struct AppState {
    child: Mutex<Option<Child>>,
    exiting: AtomicBool,
    /// 环境检测缓存（node/dsh 路径启动后不会变，避免每次进入控制台页都起子进程探测）。
    env_info: Mutex<Option<EnvInfo>>,
}

/// 托盘「全屏 / 退出全屏」菜单项句柄（文案随主窗口全屏状态切换）。
static FS_MENU_ITEM: OnceLock<MenuItem<tauri::Wry>> = OnceLock::new();

// ---------------------------------------------------------------- 序列化类型

#[derive(Clone, Serialize)]
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

#[derive(Debug, Serialize)]
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

/// 根据主窗口当前全屏状态，同步托盘「全屏 / 退出全屏」菜单文案。
fn sync_fs_menu_label(app: &tauri::AppHandle) {
    let Some(item) = FS_MENU_ITEM.get() else { return };
    let is_fs = app
        .get_webview_window("main")
        .and_then(|win| win.is_fullscreen().ok())
        .unwrap_or(false);
    let _ = item.set_text(if is_fs { "退出全屏" } else { "全屏" });
}

/// 切换主窗口全屏状态，并同步托盘菜单文案（托盘菜单与快捷键共用）。
fn toggle_fullscreen_app(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let is_fs = win.is_fullscreen().unwrap_or(false);
        let _ = win.set_fullscreen(!is_fs);
        sync_fs_menu_label(app);
    }
}

// 全屏快捷键（F11 切换 / Esc 退出）：通过 initialization_script 注入，
// 每次页面加载都会执行（含整窗导航后的 Harness 远程页），解决主屏全屏时托盘不可达的问题。
// Esc 仅在事件未被页面自身处理（未 preventDefault / 未 stopPropagation）
// 且焦点不在输入控件内时生效，优先让 Harness 界面使用 Esc。
// 走核心窗口命令（getCurrentWindow().setFullscreen）；远程页（Harness）通过
// capabilities/harness-remote.json 获得 core:window:allow-*-fullscreen 权限。
// WebView2 无浏览器自带 F11/Esc 全屏行为，必须由脚本自行处理。
const SHORTCUT_SCRIPT: &str = r#"(() => {
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

#[cfg(target_os = "windows")]
fn apply_window_effects(window: &tauri::WebviewWindow) {
    match window_vibrancy::apply_mica(window, Some(true)) {
        Ok(()) => {}
        Err(e) => eprintln!("[dsh-desktop] Mica 背景应用失败（非致命）: {e}"),
    }
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let back = MenuItem::with_id(app, "back", "返回控制台", true, None::<&str>)?;
    let fs = MenuItem::with_id(app, "fullscreen", "全屏", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出应用", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&back, &fs, &quit])?;
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
                    let _ = win.navigate(tauri::Url::parse("http://tauri.localhost/").expect("应用页 URL 解析失败"));
                }
            }
            "fullscreen" => toggle_fullscreen_app(app),
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

/// 环境检测：结果缓存于 AppState（force=true 时强制重新探测，用于“重试”按钮）。
#[tauri::command(async)]
fn get_env_info(state: State<'_, AppState>, force: Option<bool>) -> EnvInfo {
    if !force.unwrap_or(false) {
        if let Some(cached) = state.env_info.lock().unwrap().as_ref() {
            return cached.clone();
        }
    }
    let node = where_find("node");
    let dsh = where_find("dsh");
    let dsh_entry = dsh.as_deref().and_then(resolve_dsh_entry);
    let info = EnvInfo {
        node,
        dsh,
        dsh_entry,
    };
    *state.env_info.lock().unwrap() = Some(info.clone());
    info
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigInfo {
    web_port: u16,
    web_url: String,
    auto_open_devtools: bool,
    auto_start: bool,
}

#[tauri::command]
fn get_config(cfg: State<'_, AppConfig>) -> ConfigInfo {
    ConfigInfo {
        web_port: cfg.web.port,
        web_url: cfg.web_url(),
        auto_open_devtools: cfg.devtools.auto_open,
        auto_start: cfg.service.auto_start,
    }
}

#[tauri::command(async)]
fn get_status(
    state: State<'_, AppState>,
    cfg: State<'_, AppConfig>,
) -> StatusInfo {
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
        port_in_use: port_in_use(cfg.web.port),
        owned,
        url: cfg.web_url(),
    }
}

/// 服务启动核心逻辑（命令、托盘、自动启动共用）。
fn start_service_inner(
    app: &tauri::AppHandle,
    child_state: &Mutex<Option<Child>>,
    cfg: &AppConfig,
) -> Result<StartOutcome, StartError> {
    let web_url = cfg.web_url();
    let port = cfg.web.port;
    let timeout = Duration::from_secs(cfg.service.start_timeout_secs.max(5));

    // 1) 环境检测：node 与全局 dsh 必须存在
    let node = where_find("node").ok_or(StartError::NodeMissing)?;
    let dsh_shim = where_find("dsh").ok_or(StartError::DshMissing)?;

    // 2) 端口占用检测：已被占用 → 视为已有实例，不重复启动
    if port_in_use(port) {
        return Ok(StartOutcome::AlreadyRunning { url: web_url });
    }

    // 清理可能残留的旧进程
    if let Some(mut old) = child_state.lock().unwrap().take() {
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
    cmd.arg("web").arg("--port").arg(port.to_string());

    // 不弹出控制台窗口：GUI 子系统应用 spawn 控制台子系统进程（node/cmd）时，
    // Windows 默认会新建一个终端窗口；CREATE_NO_WINDOW 禁止之，输出仍写入日志文件。
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW

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
    *child_state.lock().unwrap() = Some(child);

    // 4) 等待配置端口就绪
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if port_in_use(port) {
            return Ok(StartOutcome::Started { url: web_url });
        }
        {
            let mut guard = child_state.lock().unwrap();
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
        timeout.as_secs(),
        log_file.display()
    )))
}

#[tauri::command(async)]
fn start_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    cfg: State<'_, AppConfig>,
) -> Result<StartOutcome, StartError> {
    start_service_inner(&app, &state.child, &cfg)
}

#[tauri::command(async)]
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
    let cfg = AppConfig::load();
    let setup_cfg = cfg.clone();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 第二个实例启动时，唤醒已有窗口
            show_main(app);
        }))
        .manage(AppState {
            child: Mutex::new(None),
            exiting: AtomicBool::new(false),
            env_info: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_env_info,
            get_config,
            get_status,
            start_service,
            stop_service
        ])
        .setup(move |app| {
            app.manage(setup_cfg.clone());

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
            apply_window_effects(&win);

            // 按配置决定是否自动打开 DevTools（调试用）
            #[cfg(any(debug_assertions, feature = "devtools"))]
            if setup_cfg.devtools.auto_open {
                let _ = win.open_devtools();
            }

            build_tray(app.handle())?;
            sync_fs_menu_label(app.handle());

            // 自动启动服务（配置 service.autoStart，默认开）
            if setup_cfg.service.auto_start {
                let app = app.handle().clone();
                std::thread::spawn(move || {
                    let cfg = app.state::<AppConfig>();
                    let st = app.state::<AppState>();
                    if let Err(e) = start_service_inner(&app, &st.child, &cfg) {
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
                        if !window.app_handle().state::<AppState>().exiting.load(Ordering::SeqCst) {
                            api.prevent_close();
                            let _ = window.hide();
                        }
                    }
                    // 进入/退出全屏会触发窗口尺寸变化：借此同步托盘菜单文案
                    WindowEvent::Resized(_) => sync_fs_menu_label(window.app_handle()),
                    _ => {}
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
