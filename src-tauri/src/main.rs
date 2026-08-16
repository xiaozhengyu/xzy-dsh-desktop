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
    io::{Read, Seek, SeekFrom},
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
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
struct ThemeConfig {
    dark: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self { dark: true }
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
struct AppConfig {
    note: String,
    web: WebConfig,
    service: ServiceConfig,
    devtools: DevtoolsConfig,
    theme: ThemeConfig,
    /// 开机自启状态（注册表镜像，load 时以真实注册表状态覆盖，供 UI 显示）。
    autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            note: "DSH 桌面端配置文件。修改后重启应用生效。web.port 为 dsh web 服务端口（默认 3081，避开 Harness 默认 3080 可与现有会话并存）；web.host 为绑定主机；service.startTimeoutSecs 为服务启动等待上限（秒）；service.autoStart 为启动应用时自动拉起服务；devtools.autoOpen 为调试用自动打开开发者工具；theme.dark 为界面深浅主题；autostart 为开机自启状态（注册表镜像）。".into(),
            web: WebConfig::default(),
            service: ServiceConfig::default(),
            devtools: DevtoolsConfig::default(),
            theme: ThemeConfig::default(),
            autostart: false,
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
        let mut cfg = match Self::config_path() {
            Some(file) => {
                if let Some(dir) = file.parent() {
                    let _ = fs::create_dir_all(dir);
                }
                if !file.exists() {
                    if let Ok(json) = serde_json::to_string_pretty(&def) {
                        let _ = fs::write(&file, json);
                    }
                    def
                } else {
                    match fs::read_to_string(&file)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                    {
                        Some(c) => c,
                        None => {
                            eprintln!("[dsh-desktop] 配置文件解析失败，使用默认值: {}", file.display());
                            def
                        }
                    }
                }
            }
            None => def,
        };
        // 开机自启状态以注册表为准（配置仅作镜像，供 UI 显示开关状态）
        cfg.autostart = reg_autostart_enabled();
        cfg
    }

    /// 持久化当前配置到 config.json（保留 note 字段）。
    fn save(&self) -> Result<(), String> {
        let Some(file) = Self::config_path() else {
            return Err("无法确定配置文件路径".into());
        };
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&file, json).map_err(|e| format!("写入配置失败: {e}"))
    }
}

struct AppState {
    child: Mutex<Option<Child>>,
    exiting: AtomicBool,
    /// 环境检测缓存（node/dsh 路径启动后不会变，避免每次进入控制台页都起子进程探测）。
    env_info: Mutex<Option<EnvInfo>>,
    /// 配置（运行时可变：set_config 修改后同时持久化到 config.json）。
    config: Mutex<AppConfig>,
    /// 本应用派生的 dsh 进程启动时刻（用于前端展示运行时长）。
    started_at: Mutex<Option<SystemTime>>,
    /// 检查更新结果缓存（10 分钟内不重复联网）。
    update_cache: Mutex<Option<(Instant, Option<UpdateInfo>)>>,
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

/// dsh 更新检查结果（P1）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    current: String,
    latest: String,
    has_update: bool,
}

// ---------------------------------------------------------------- 系统工具

/// 在 PATH 中查找可执行文件，返回第一个带扩展名（.cmd/.exe）的条目。
fn where_find(exe: &str) -> Option<String> {
    let mut cmd = Command::new("where");
    cmd.arg(exe);
    // 隐藏控制台窗口：GUI 子系统应用 spawn 控制台进程（where.exe）时会新建终端窗口
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let out = cmd.output().ok()?;
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
    let mut cmd = Command::new("taskkill");
    cmd.arg("/PID").arg(pid.to_string()).arg("/T").arg("/F");
    // 隐藏控制台窗口：taskkill 为控制台进程，spawn 时会新建终端窗口
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let _ = cmd.output();
}

// ---------------------------------------------------------------- 开机自启（注册表）

const AUTOSTART_RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_VALUE: &str = "DSH-Desktop";

/// 读取注册表 Run 键中的自启项是否存在。
fn reg_autostart_enabled() -> bool {
    let mut cmd = Command::new("reg");
    cmd.args(["query", AUTOSTART_RUN_KEY, "/v", AUTOSTART_VALUE]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

/// 写入/删除注册表 Run 键自启项（值为当前 exe 路径）。
fn reg_set_autostart(enabled: bool) -> Result<(), String> {
    let mut cmd = Command::new("reg");
    if enabled {
        let exe = std::env::current_exe().map_err(|e| format!("无法定位当前 exe: {e}"))?;
        cmd.args([
            "add", AUTOSTART_RUN_KEY, "/v", AUTOSTART_VALUE, "/t", "REG_SZ", "/d",
            &format!("\"{}\"", exe.display()), "/f",
        ]);
    } else {
        cmd.args(["delete", AUTOSTART_RUN_KEY, "/v", AUTOSTART_VALUE, "/f"]);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let out = cmd.output().map_err(|e| format!("reg 执行失败: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("reg 操作失败（exit {}）", out.status.code().unwrap_or(-1)))
    }
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
fn apply_window_effects(window: &tauri::WebviewWindow, dark: bool) {
    match window_vibrancy::apply_mica(window, Some(dark)) {
        Ok(()) => {}
        Err(e) => eprintln!("[dsh-desktop] Mica 背景应用失败（非致命）: {e}"),
    }
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let back = MenuItem::with_id(app, "back", "控制台", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let fs = MenuItem::with_id(app, "fullscreen", "全屏", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出应用", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&back, &settings, &fs, &quit])?;
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
    web_host: String,
    web_url: String,
    auto_open_devtools: bool,
    auto_start: bool,
    theme_dark: bool,
    autostart: bool,
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> ConfigInfo {
    let cfg = state.config.lock().unwrap();
    ConfigInfo {
        web_port: cfg.web.port,
        web_host: cfg.web.host.clone(),
        web_url: cfg.web_url(),
        auto_open_devtools: cfg.devtools.auto_open,
        auto_start: cfg.service.auto_start,
        theme_dark: cfg.theme.dark,
        autostart: cfg.autostart,
    }
}

#[tauri::command(async)]
fn get_status(state: State<'_, AppState>) -> StatusInfo {
    let cfg = state.config.lock().unwrap().clone();
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
    state: &AppState,
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
    *state.child.lock().unwrap() = Some(child);

    // 4) 等待配置端口就绪
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if port_in_use(port) {
            *state.started_at.lock().unwrap() = Some(SystemTime::now());
            return Ok(StartOutcome::Started { url: web_url });
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
        timeout.as_secs(),
        log_file.display()
    )))
}

#[tauri::command(async)]
fn start_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<StartOutcome, StartError> {
    let cfg = state.config.lock().unwrap().clone();
    start_service_inner(&app, &state, &cfg)
}

#[tauri::command(async)]
fn stop_service(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let child = state.child.lock().unwrap().take();
    if let Some(mut c) = child {
        kill_tree(c.id());
        let _ = c.wait();
    }
    *state.started_at.lock().unwrap() = None;
    append_log_line(&app, "[dsh-desktop] --- normal shutdown ---");
    Ok(())
}

/// 往 dsh-web.log 追加一行（正常退出标记用）。
fn append_log_line(app: &tauri::AppHandle, line: &str) {
    use std::io::Write;
    if let Ok(dir) = app.path().app_log_dir() {
        let _ = fs::create_dir_all(&dir);
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("dsh-web.log"))
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

// ---------------------------------------------------------------- 控制台信息 / 日志

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceInfo {
    pid: Option<u32>,
    started_at_ms: Option<u64>,
    log_path: String,
}

/// 服务详情：PID、启动时刻（epoch 毫秒，供前端计算运行时长）、日志文件路径。
#[tauri::command]
fn get_service_info(app: tauri::AppHandle, state: State<'_, AppState>) -> ServiceInfo {
    let owned = {
        let mut guard = state.child.lock().unwrap();
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    *guard = None;
                    false
                }
                _ => true,
            }
        } else {
            false
        }
    };
    let pid = if owned {
        state.child.lock().unwrap().as_ref().map(|c| c.id())
    } else {
        None
    };
    let started_at_ms = if owned {
        state
            .started_at
            .lock()
            .unwrap()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
    } else {
        None
    };
    let log_path = app
        .path()
        .app_log_dir()
        .map(|d| d.join("dsh-web.log").to_string_lossy().into_owned())
        .unwrap_or_default();
    ServiceInfo { pid, started_at_ms, log_path }
}

/// 在资源管理器中定位日志文件（explorer /select）。
#[tauri::command]
fn open_log_folder(app: tauri::AppHandle) -> Result<(), String> {
    let log_path = app
        .path()
        .app_log_dir()
        .map_err(|e| e.to_string())?
        .join("dsh-web.log");
    let mut cmd = Command::new("explorer");
    cmd.arg("/select,").arg(&log_path);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd.spawn().map_err(|e| format!("无法打开资源管理器: {e}"))?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TailResult {
    offset: u64,
    lines: Vec<String>,
    truncated: bool,
}

/// 增量读取 dsh-web.log：从上次 offset 读到文件尾。
/// 文件被截断（如被清空/轮转）时从头读取并置 truncated。
#[tauri::command]
fn tail_log(app: tauri::AppHandle, offset: u64) -> TailResult {
    let log_path = app
        .path()
        .app_log_dir()
        .map(|d| d.join("dsh-web.log"))
        .unwrap_or_default();
    let size = fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return TailResult { offset: 0, lines: vec![], truncated: false };
    }
    let truncated = offset > size;
    let start = if truncated { 0 } else { offset };
    let mut content = String::new();
    if let Ok(mut f) = File::open(&log_path) {
        let _ = f.seek(SeekFrom::Start(start));
        let _ = f.read_to_string(&mut content);
    }
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    if start > 0 && !lines.is_empty() {
        // 从任意字节偏移开始可能落在行中间：丢弃首个不完整片段，避免重复/乱码
        lines.remove(0);
    }
    lines.retain(|l| !l.is_empty());
    TailResult { offset: size, lines, truncated }
}

/// 清空日志文件（Node 以 append 方式写入，truncate 后继续追加安全）。
#[tauri::command]
fn clear_log(app: tauri::AppHandle) -> Result<(), String> {
    let log_path = app
        .path()
        .app_log_dir()
        .map_err(|e| e.to_string())?
        .join("dsh-web.log");
    fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .map_err(|e| format!("无法清空日志: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------- P1：重启 / 诊断 / 更新

/// 一键重启：仅当服务由本应用托管时允许；外部占用直接拒绝。
#[tauri::command(async)]
fn restart_service(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<StartOutcome, String> {
    let cfg = state.config.lock().unwrap().clone();
    let owned = {
        let mut guard = state.child.lock().unwrap();
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    *guard = None;
                    false
                }
                _ => true,
            }
        } else {
            false
        }
    };
    if !owned {
        return Err("服务由外部进程占用，无法重启（请在外部停止该进程后再试）".into());
    }
    let child = state.child.lock().unwrap().take();
    if let Some(mut c) = child {
        kill_tree(c.id());
        let _ = c.wait();
    }
    *state.started_at.lock().unwrap() = None;
    start_service_inner(&app, &state, &cfg).map_err(|e| format!("重启失败: {e:?}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagItem {
    check: String,
    ok: bool,
    detail: String,
}

/// 自检诊断：node/dsh、端口占用归属、日志目录可写、配置文件可读。
#[tauri::command]
fn run_diagnostics(app: tauri::AppHandle, state: State<'_, AppState>) -> Vec<DiagItem> {
    let cfg = state.config.lock().unwrap().clone();
    let mut items = Vec::new();

    match where_find("node") {
        Some(p) => items.push(DiagItem { check: "Node.js".into(), ok: true, detail: p }),
        None => items.push(DiagItem {
            check: "Node.js".into(),
            ok: false,
            detail: "PATH 中未找到 node，请先安装 Node.js".into(),
        }),
    }
    match where_find("dsh") {
        Some(shim) => {
            let detail = match resolve_dsh_entry(&shim) {
                Some(entry) => format!("{shim} → {entry}"),
                None => format!("{shim}（未解析出 JS 入口，将回退 cmd /C dsh）"),
            };
            items.push(DiagItem { check: "dsh".into(), ok: true, detail });
        }
        None => items.push(DiagItem {
            check: "dsh".into(),
            ok: false,
            detail: "PATH 中未找到 dsh，请执行 npm install -g @deepseek/dsh".into(),
        }),
    }
    let port = cfg.web.port;
    let occupied = port_in_use(port);
    let owned = state.child.lock().unwrap().as_ref().is_some();
    items.push(match (occupied, owned) {
        (false, _) => DiagItem { check: format!("端口 {port}"), ok: true, detail: "空闲".into() },
        (true, true) => DiagItem {
            check: format!("端口 {port}"),
            ok: true,
            detail: "由本应用托管（正常运行）".into(),
        },
        (true, false) => DiagItem {
            check: format!("端口 {port}"),
            ok: false,
            detail: "被外部进程占用——可能是异常强杀残留，可在控制台执行「清理残留」".into(),
        },
    });
    match app.path().app_log_dir() {
        Ok(dir) => {
            let writable = fs::create_dir_all(&dir).is_ok()
                && File::options()
                    .create(true)
                    .append(true)
                    .open(dir.join("dsh-web.log"))
                    .is_ok();
            items.push(DiagItem {
                check: "日志目录".into(),
                ok: writable,
                detail: if writable { dir.display().to_string() } else { format!("不可写: {}", dir.display()) },
            });
        }
        Err(e) => items.push(DiagItem { check: "日志目录".into(), ok: false, detail: e.to_string() }),
    }
    match AppConfig::config_path() {
        Some(p) => {
            let ok = fs::read_to_string(&p)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .is_some();
            items.push(DiagItem {
                check: "配置文件".into(),
                ok,
                detail: if ok { p.display().to_string() } else { format!("缺失或解析失败: {}", p.display()) },
            });
        }
        None => items.push(DiagItem {
            check: "配置文件".into(),
            ok: false,
            detail: "无法确定路径（APPDATA 未设置）".into(),
        }),
    }
    items
}

/// 解析字符串中的首个 x.y.z 形态版本号（宽松匹配，容忍 "v1.2.3" / "dsh 0.5.0" 等）。
fn extract_version(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_digit() && c != '.')
        .find(|p| {
            let parts: Vec<&str> = p.split('.').collect();
            parts.len() >= 2
                && parts.iter().all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()))
        })
        .map(|s| s.to_string())
}

fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim_start_matches('v').split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().unwrap_or("0").parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    Some((a, b, c))
}

/// 简单 semver 比较（major.minor.patch 数值比较）。
fn semver_gt(a: &str, b: &str) -> bool {
    match (parse_semver(a), parse_semver(b)) {
        (Some(x), Some(y)) => x > y,
        _ => false,
    }
}

/// 本地 dsh 版本（优先 node <入口> --version，回退 cmd /C dsh --version）。
fn dsh_version() -> Option<String> {
    let node = where_find("node")?;
    let shim = where_find("dsh")?;
    let mut cmd = match resolve_dsh_entry(&shim) {
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
    cmd.arg("--version");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let out = cmd.output().ok()?;
    extract_version(&String::from_utf8_lossy(&out.stdout))
}

/// 带超时的子进程执行（捕获 stdout/stderr；超时则杀掉返回 None）。
fn run_cmd_timeout(cmd: &mut Command, timeout: Duration) -> Option<std::process::Output> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = cmd.spawn().ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    child.wait_with_output().ok()
}

/// 检查 dsh 是否有新版本：npm view @deepseek/dsh version 与本地版本比较。
/// 失败（离线/无 npm/超时）返回 None，不打扰用户；结果缓存 10 分钟。
fn check_update_inner() -> Option<UpdateInfo> {
    let current = dsh_version()?;
    let mut cmd = Command::new("npm");
    cmd.args(["view", "@deepseek/dsh", "version"]);
    let out = run_cmd_timeout(&mut cmd, Duration::from_secs(8))?;
    if !out.status.success() {
        return None;
    }
    let latest = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if latest.is_empty() {
        return None;
    }
    let has_update = semver_gt(&latest, &current);
    Some(UpdateInfo { current, latest, has_update })
}

#[tauri::command(async)]
fn check_update(state: State<'_, AppState>) -> Option<UpdateInfo> {
    {
        let cache = state.update_cache.lock().unwrap();
        if let Some((at, res)) = cache.as_ref() {
            if at.elapsed() < Duration::from_secs(600) {
                return res.clone();
            }
        }
    }
    let result = check_update_inner();
    *state.update_cache.lock().unwrap() = Some((Instant::now(), result.clone()));
    result
}

// ---------------------------------------------------------------- P2：设置

/// 修改配置（写回 config.json，下次启动服务生效的项由 UI 文案提示）。
#[tauri::command]
fn set_config(
    state: State<'_, AppState>,
    web_port: Option<u16>,
    web_host: Option<String>,
    auto_start: Option<bool>,
    theme_dark: Option<bool>,
    auto_open_devtools: Option<bool>,
) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    if let Some(p) = web_port {
        cfg.web.port = p;
    }
    if let Some(h) = web_host {
        let h = h.trim().trim_matches('"').to_string();
        cfg.web.host = if h.is_empty() { "127.0.0.1".into() } else { h };
    }
    if let Some(a) = auto_start {
        cfg.service.auto_start = a;
    }
    if let Some(t) = theme_dark {
        cfg.theme.dark = t;
    }
    if let Some(d) = auto_open_devtools {
        cfg.devtools.auto_open = d;
    }
    cfg.save()
}

/// 开机自启（注册表 Run 键 + 配置镜像）。
#[tauri::command]
fn set_autostart(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    reg_set_autostart(enabled)?;
    let mut cfg = state.config.lock().unwrap();
    cfg.autostart = enabled;
    let _ = cfg.save(); // 镜像写入失败不阻断（注册表为准）
    Ok(())
}

/// 重应用 Mica 深浅主题（失败降级：仅前端 CSS 变量切换生效）。
#[tauri::command]
fn apply_theme(window: tauri::WebviewWindow, dark: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        match window_vibrancy::apply_mica(&window, Some(dark)) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("[dsh-desktop] Mica 重应用失败（非致命）: {e}");
                Ok(())
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, dark);
        Ok(())
    }
}

/// 配置文件路径（设置页「关于」展示）。
#[tauri::command]
fn get_config_path() -> Option<String> {
    AppConfig::config_path().map(|p| p.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------- P3：异常退出兜底

/// 异常退出检测：服务未运行、日志非空、且尾部无正常退出标记 → 判定上次异常退出。
#[tauri::command]
fn check_abnormal_exit(app: tauri::AppHandle, state: State<'_, AppState>) -> bool {
    let cfg = state.config.lock().unwrap().clone();
    if port_in_use(cfg.web.port) {
        return false;
    }
    let Some(dir) = app.path().app_log_dir().ok() else {
        return false;
    };
    let Ok(content) = fs::read_to_string(dir.join("dsh-web.log")) else {
        return false;
    };
    if content.trim().is_empty() {
        return false;
    }
    !content.contains("--- normal shutdown ---")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CleanResult {
    cleaned: bool,
    detail: String,
}

/// 一键清理残留进程：netstat 定位占用端口的监听 PID → taskkill /T /F。
#[tauri::command]
fn clean_stale(state: State<'_, AppState>) -> Result<CleanResult, String> {
    let cfg = state.config.lock().unwrap().clone();
    let port = cfg.web.port;
    if state.child.lock().unwrap().as_ref().is_some() {
        return Err("服务由本应用托管，无需清理".into());
    }
    if !port_in_use(port) {
        return Ok(CleanResult { cleaned: false, detail: "未发现端口占用，无残留进程".into() });
    }
    let mut cmd = Command::new("netstat");
    cmd.args(["-ano", "-p", "TCP"]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let out = cmd.output().map_err(|e| format!("netstat 执行失败: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut pids: Vec<u32> = Vec::new();
    let suffix = format!(":{port}");
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // 格式: Proto  Local Address  Foreign Address  State  PID
        if parts.len() >= 5
            && parts[1].ends_with(&suffix)
            && parts[3] == "LISTENING"
        {
            if let Ok(pid) = parts[4].parse::<u32>() {
                pids.push(pid);
            }
        }
    }
    if pids.is_empty() {
        return Ok(CleanResult {
            cleaned: false,
            detail: "端口被占用但未定位到监听进程（可能是非 TCP 监听或权限受限）".into(),
        });
    }
    for pid in &pids {
        kill_tree(*pid);
    }
    std::thread::sleep(Duration::from_millis(600));
    if port_in_use(port) {
        Ok(CleanResult {
            cleaned: false,
            detail: "已结束进程但端口仍被占用（可能还有其他进程绑定同一端口）".into(),
        })
    } else {
        let list = pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        Ok(CleanResult { cleaned: true, detail: format!("已清理残留进程: {list}") })
    }
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
            config: Mutex::new(cfg.clone()),
            started_at: Mutex::new(None),
            update_cache: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_env_info,
            get_config,
            get_status,
            get_service_info,
            open_log_folder,
            tail_log,
            clear_log,
            start_service,
            stop_service,
            restart_service,
            run_diagnostics,
            check_update,
            set_config,
            set_autostart,
            apply_theme,
            get_config_path,
            check_abnormal_exit,
            clean_stale
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
            apply_window_effects(&win, setup_cfg.theme.dark);

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
                    let st = app.state::<AppState>();
                    let cfg = st.config.lock().unwrap().clone();
                    if let Err(e) = start_service_inner(&app, &st, &cfg) {
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
        // 退出时写正常退出标记并强杀由本应用派生的 dsh 进程树，避免端口占用残留。
        // 异常强杀（任务管理器）不会经过这里 → 日志尾部无标记，前端据此提示上次异常退出。
        if let RunEvent::Exit = event {
            append_log_line(app_handle, "[dsh-desktop] --- normal shutdown ---");
            if let Some(mut child) = app_handle.state::<AppState>().child.lock().unwrap().take() {
                kill_tree(child.id());
                let _ = child.wait();
            }
        }
    });
}
