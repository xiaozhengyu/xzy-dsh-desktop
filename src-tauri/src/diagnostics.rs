//! 自检诊断：node/dsh、端口占用归属、日志目录可写、配置文件可读。

use std::fs::{self, File};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::config::AppConfig;
use crate::env::{resolve_dsh_entry, where_find};
use crate::service::{listening_pids, process_command_line, probe_port};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagItem {
    pub check: String,
    pub ok: bool,
    pub detail: String,
}

/// 自检诊断：一次跑完环境/端口/权限检查，返回结构化结果供前端渲染与复制。
#[tauri::command]
pub fn run_diagnostics(app: AppHandle, state: State<'_, AppState>) -> Vec<DiagItem> {
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
            detail: "PATH 中未找到 dsh，请执行 npm install -g @deepseek-ai/dsh".into(),
        }),
    }
    let port = cfg.web.port;
    let occupied = probe_port(&cfg);
    let owned = state.child.lock().unwrap().as_ref().is_some();
    items.push(match (occupied, owned) {
        (false, _) => DiagItem { check: format!("端口 {port}"), ok: true, detail: "空闲".into() },
        (true, true) => DiagItem {
            check: format!("端口 {port}"),
            ok: true,
            detail: "由本应用托管（正常运行）".into(),
        },
        (true, false) => {
            let detail = match listening_pids(port).first().copied() {
                Some(p) => {
                    let identity = process_command_line(p)
                        .map(|cl| {
                            let trimmed: String = cl.chars().take(90).collect();
                            format!("；命令行：{trimmed}…")
                        })
                        .unwrap_or_default();
                    format!("被外部进程占用（PID {p}{identity}）——若为 dsh 残留可清理，非 dsh 进程会拒绝终止")
                }
                None => "被外部进程占用（未定位到监听 PID）".into(),
            };
            DiagItem { check: format!("端口 {port}"), ok: false, detail }
        }
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
