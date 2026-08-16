//! 服务生命周期：端口/进程探测、启停/重启、状态与详情命令。
//! 核心职责：托管 dsh web 进程、进程树清理、端口归属判定。

use std::{
    fs::{self, File},
    net::{TcpStream, ToSocketAddrs},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime},
};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::config::AppConfig;
use crate::env::{resolve_dsh_entry, where_find};
use crate::logging::append_log_line;
use crate::state::AppState;

// ---------------------------------------------------------------- 序列化类型

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusInfo {
    pub port_in_use: bool,
    pub owned: bool,
    pub url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum StartOutcome {
    Started { url: String },
    AlreadyRunning { url: String },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "code", content = "message")]
pub enum StartError {
    NodeMissing,
    DshMissing,
    SpawnFailed(String),
    NotReady(String),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub pid: Option<u32>,
    pub started_at_ms: Option<u64>,
    pub log_path: String,
}

// ---------------------------------------------------------------- 端口 / 进程探测

/// 检测指定 host:port 是否可连接。
fn port_in_use_at(host: &str, port: u16) -> bool {
    let text = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let Ok(mut addrs) = text.to_socket_addrs() else {
        return false;
    };
    addrs.any(|a| TcpStream::connect_timeout(&a, Duration::from_millis(250)).is_ok())
}

/// 检测端口是否被占用（回环地址 127.0.0.1 / ::1）。
fn port_in_use(port: u16) -> bool {
    port_in_use_at("127.0.0.1", port) || port_in_use_at("::1", port)
}

/// 按配置的绑定 host 探测端口：
/// - 通配 host（0.0.0.0 / ::）按回环探测（0.0.0.0 监听涵盖回环）；
/// - 具体 host 直接探测该地址。
pub fn probe_port(cfg: &AppConfig) -> bool {
    let host = cfg.web.host.trim();
    if host == "0.0.0.0" || host == "::" || host.is_empty() {
        port_in_use(cfg.web.port)
    } else {
        port_in_use_at(host, cfg.web.port)
    }
}

/// HTTP 就绪探测：TCP 之上再发一次 GET /，收到 HTTP 响应头即视为 web 服务真正响应
/// （避免「端口有监听 ≠ dsh web 可用」，如残留进程只开 TCP 未完成启动）。
fn http_ready(host: &str, port: u16) -> bool {
    use std::io::{Read, Write};
    let text = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let Ok(mut addrs) = text.to_socket_addrs() else {
        return false;
    };
    let Some(addr) = addrs.next() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(800)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(800)));
    let req = format!("GET / HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/")
}

/// 按配置 host 做 HTTP 就绪探测（通配 host 回退到回环）。
fn cfg_http_ready(cfg: &AppConfig) -> bool {
    let host = cfg.web.host.trim();
    if host == "0.0.0.0" || host == "::" || host.is_empty() {
        http_ready("127.0.0.1", cfg.web.port) || http_ready("::1", cfg.web.port)
    } else {
        http_ready(host, cfg.web.port)
    }
}

/// netstat 查找监听指定端口的进程 PID 列表（格式: Proto Local Foreign State PID）。
pub fn listening_pids(port: u16) -> Vec<u32> {
    let mut cmd = Command::new("netstat");
    cmd.args(["-ano", "-p", "TCP"]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let Ok(out) = cmd.output() else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout);
    let suffix = format!(":{port}");
    let mut pids = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 5 && parts[1].ends_with(&suffix) && parts[3] == "LISTENING" {
            if let Ok(pid) = parts[4].parse::<u32>() {
                pids.push(pid);
            }
        }
    }
    pids
}

/// 获取进程命令行（PowerShell Get-CimInstance，wmic 在新系统已弃用）。
pub fn process_command_line(pid: u32) -> Option<String> {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!("(Get-CimInstance Win32_Process -Filter \"ProcessId={pid}\").CommandLine"),
    ]);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 判断命令行是否属于 dsh web 服务进程（node <dsh 入口> web ... / cmd /C dsh web ...）。
/// 特征：命令含 dsh 标识 且 含 web 子命令。无法确认时返回 false（宁可不清，不可误杀）。
pub fn is_dsh_process(cmdline: &str) -> bool {
    let lower = cmdline.to_lowercase();
    let has_dsh_marker =
        lower.contains("dsh") || lower.contains("@deepseek") || lower.contains("bin.js");
    has_dsh_marker && lower.contains(" web ")
}

/// 用 Windows 原生 taskkill 强杀进程树（含子进程），防止端口残留。
pub fn kill_tree(pid: u32) {
    let mut cmd = Command::new("taskkill");
    cmd.arg("/PID").arg(pid.to_string()).arg("/T").arg("/F");
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let _ = cmd.output();
}

// ---------------------------------------------------------------- 状态命令

#[tauri::command(async)]
pub fn get_status(state: State<'_, AppState>) -> StatusInfo {
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
        port_in_use: probe_port(&cfg),
        owned,
        url: cfg.web_url(),
    }
}

/// 服务详情：PID、启动时刻（epoch 毫秒，供前端计算运行时长）、日志文件路径。
/// 非托管的外部实例也能通过 netstat 探测到占用 PID。
#[tauri::command]
pub fn get_service_info(app: AppHandle, state: State<'_, AppState>) -> ServiceInfo {
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
    let cfg = state.config.lock().unwrap().clone();
    let pid = if owned {
        state.child.lock().unwrap().as_ref().map(|c| c.id())
    } else if probe_port(&cfg) {
        listening_pids(cfg.web.port).first().copied()
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

// ---------------------------------------------------------------- 启停流程

/// 服务启动核心逻辑（命令、自动启动共用）。
pub fn start_service_inner(
    app: &AppHandle,
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
    if probe_port(cfg) {
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
    // 绑定 host 闭环：配置了非默认 host 时显式传给 dsh web
    if cfg.web.host != "127.0.0.1" && !cfg.web.host.is_empty() {
        cmd.arg("--host").arg(&cfg.web.host);
    }

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

    // 4) 等待服务就绪：TCP 可连 且 HTTP 返回响应（比单纯端口探测更接近「可用」）
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if probe_port(cfg) && cfg_http_ready(cfg) {
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
pub fn start_service(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<StartOutcome, StartError> {
    let cfg = state.config.lock().unwrap().clone();
    start_service_inner(&app, &state, &cfg)
}

#[tauri::command(async)]
pub fn stop_service(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let child = state.child.lock().unwrap().take();
    if let Some(mut c) = child {
        kill_tree(c.id());
        let _ = c.wait();
    }
    *state.started_at.lock().unwrap() = None;
    append_log_line(&app, "[dsh-desktop] --- normal shutdown ---");
    Ok(())
}

/// 一键重启：仅当服务由本应用托管时允许；外部占用直接拒绝。
#[tauri::command(async)]
pub fn restart_service(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<StartOutcome, String> {
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
