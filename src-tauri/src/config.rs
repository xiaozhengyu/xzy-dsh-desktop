//! 配置（config.json）：结构、读写、注册表自启、主题解析与设置命令。

use std::{fs, process::Command};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

// ---------------------------------------------------------------- 配置结构

/// Web 服务绑定配置。
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct WebConfig {
    pub host: String,
    pub port: u16,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self { host: "127.0.0.1".into(), port: 3081 }
    }
}

/// 服务启动相关配置。
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct ServiceConfig {
    pub start_timeout_secs: u64,
    pub auto_start: bool,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self { start_timeout_secs: 25, auto_start: true }
    }
}

/// 调试用配置。
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct DevtoolsConfig {
    pub auto_open: bool,
}

impl Default for DevtoolsConfig {
    fn default() -> Self {
        Self { auto_open: false }
    }
}

/// 主题模式：跟随系统 / 浅色 / 深色。
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::System
    }
}

/// 主题配置。
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct ThemeConfig {
    pub mode: ThemeMode,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self { mode: ThemeMode::default() }
    }
}

/// 顶层配置。首次运行在 %APPDATA%\com.deepseek.harness-desktop\config.json 生成。
#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct AppConfig {
    pub note: String,
    pub web: WebConfig,
    pub service: ServiceConfig,
    pub devtools: DevtoolsConfig,
    pub theme: ThemeConfig,
    /// 开机自启状态（注册表镜像，load 时以真实注册表状态覆盖，供 UI 显示）。
    pub autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            note: "DSH 桌面端配置文件。修改后重启应用生效。web.port 为 dsh web 服务端口（默认 3081，避开 Harness 默认 3080 可与现有会话并存）；web.host 为绑定主机；service.startTimeoutSecs 为服务启动等待上限（秒）；service.autoStart 为启动应用时自动拉起服务；devtools.autoOpen 为调试用自动打开开发者工具；theme.mode 为界面主题（system/light/dark）；autostart 为开机自启状态（注册表镜像）。".into(),
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
    pub fn web_url(&self) -> String {
        format!("http://{}:{}", self.web.host, self.web.port)
    }

    /// 配置文件路径（无需 AppHandle，供窗口创建前读取）。
    pub fn config_path() -> Option<std::path::PathBuf> {
        std::env::var("APPDATA").ok().map(|a| {
            std::path::PathBuf::from(a)
                .join("com.deepseek.harness-desktop")
                .join("config.json")
        })
    }

    /// 读取配置文件；不存在则写入默认值；解析失败回退默认值。
    pub fn load() -> Self {
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

    /// 持久化当前配置到 config.json。
    pub fn save(&self) -> Result<(), String> {
        let Some(file) = Self::config_path() else {
            return Err("无法确定配置文件路径".into());
        };
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&file, json).map_err(|e| format!("写入配置失败: {e}"))
    }
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

// ---------------------------------------------------------------- 主题解析

/// 读取 Windows 系统深色模式（HKCU\...\Themes\Personalize\AppsUseLightTheme）。
fn system_dark() -> bool {
    let mut cmd = Command::new("reg");
    cmd.args([
        "query",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "/v",
        "AppsUseLightTheme",
    ]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let light = cmd
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .find_map(|l| l.split_whitespace().last().map(|s| s.to_string()))
        })
        .map(|v| v == "0x1")
        .unwrap_or(true); // 读取失败时按浅色处理
    !light
}

/// 按主题模式解析最终深浅（启动时应用 Mica 用）。
pub fn resolve_theme_dark(mode: &ThemeMode) -> bool {
    match mode {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => system_dark(),
    }
}

fn theme_mode_str(mode: &ThemeMode) -> String {
    match mode {
        ThemeMode::System => "system".into(),
        ThemeMode::Light => "light".into(),
        ThemeMode::Dark => "dark".into(),
    }
}

// ---------------------------------------------------------------- 设置命令

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInfo {
    pub web_port: u16,
    pub web_host: String,
    pub web_url: String,
    pub auto_open_devtools: bool,
    pub auto_start: bool,
    pub theme_mode: String,
    pub autostart: bool,
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> ConfigInfo {
    let cfg = state.config.lock().unwrap();
    ConfigInfo {
        web_port: cfg.web.port,
        web_host: cfg.web.host.clone(),
        web_url: cfg.web_url(),
        auto_open_devtools: cfg.devtools.auto_open,
        auto_start: cfg.service.auto_start,
        theme_mode: theme_mode_str(&cfg.theme.mode),
        autostart: cfg.autostart,
    }
}

/// 修改配置（写回 config.json，下次启动服务生效的项由 UI 文案提示）。
#[tauri::command]
pub fn set_config(
    state: State<'_, AppState>,
    web_port: Option<u16>,
    web_host: Option<String>,
    auto_start: Option<bool>,
    theme_mode: Option<String>,
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
    if let Some(m) = theme_mode {
        cfg.theme.mode = match m.as_str() {
            "light" => ThemeMode::Light,
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::System,
        };
    }
    if let Some(d) = auto_open_devtools {
        cfg.devtools.auto_open = d;
    }
    cfg.save()
}

/// 开机自启（注册表 Run 键 + 配置镜像）。
#[tauri::command]
pub fn set_autostart(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    reg_set_autostart(enabled)?;
    let mut cfg = state.config.lock().unwrap();
    cfg.autostart = enabled;
    let _ = cfg.save(); // 镜像写入失败不阻断（注册表为准）
    Ok(())
}

/// 重应用 Mica 深浅主题（失败降级：仅前端 CSS 变量切换生效）。
#[tauri::command]
pub fn apply_theme(window: tauri::WebviewWindow, dark: bool) -> Result<(), String> {
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
pub fn get_config_path() -> Option<String> {
    AppConfig::config_path().map(|p| p.to_string_lossy().into_owned())
}
