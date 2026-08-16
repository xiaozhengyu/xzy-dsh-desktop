//! 残留兜底：异常退出 / 外部占用检测，以及带进程身份校验的一键清理。

use std::fs;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::service::{is_dsh_process, kill_tree, listening_pids, probe_port, process_command_line};
use crate::state::AppState;

/// 残留状态（控制台加载时探测一次）：上次异常退出 / 端口被外部进程占用。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleInfo {
    /// 服务未运行且日志尾部无正常退出标记（上次异常强杀）。
    pub abnormal_exit: bool,
    /// 端口被非本应用托管的进程占用（可能为残留，也可能是外部正常实例）。
    pub external_occupied: bool,
    /// 外部占用进程的 PID（netstat 探测）。
    pub port_pid: Option<u32>,
}

/// 残留状态检测：区分「异常退出」与「外部占用」两种可清理场景。
#[tauri::command]
pub fn check_stale_info(app: AppHandle, state: State<'_, AppState>) -> StaleInfo {
    let cfg = state.config.lock().unwrap().clone();
    // 本应用托管运行中 → 无残留问题（与 get_status 一致：死进程句柄先清理）
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
    if owned {
        return StaleInfo { abnormal_exit: false, external_occupied: false, port_pid: None };
    }
    let content = app
        .path()
        .app_log_dir()
        .ok()
        .map(|d| d.join("dsh-web.log"))
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    let has_normal_mark = content.contains("--- normal shutdown ---");
    let log_nonempty = !content.trim().is_empty();

    let occupied = probe_port(&cfg);
    StaleInfo {
        // 服务未运行 + 日志非空 + 无正常退出标记 → 上次异常强杀
        abnormal_exit: !occupied && log_nonempty && !has_normal_mark,
        // 外部进程占着端口（且非本应用托管）
        external_occupied: occupied,
        port_pid: if occupied { listening_pids(cfg.web.port).first().copied() } else { None },
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub cleaned: bool,
    pub detail: String,
}

/// 一键清理残留进程：netstat 定位占用端口的监听 PID → 校验进程身份（确认为
/// dsh web 服务）→ taskkill /T /F。非 dsh 进程拒绝终止，避免误杀无关服务。
#[tauri::command]
pub fn clean_stale(state: State<'_, AppState>) -> Result<CleanResult, String> {
    let cfg = state.config.lock().unwrap().clone();
    let port = cfg.web.port;
    if state.child.lock().unwrap().as_ref().is_some() {
        return Err("服务由本应用托管，无需清理".into());
    }
    if !probe_port(&cfg) {
        return Ok(CleanResult { cleaned: false, detail: "未发现端口占用，无残留进程".into() });
    }
    let pids = listening_pids(port);
    if pids.is_empty() {
        return Ok(CleanResult {
            cleaned: false,
            detail: "端口被占用但未定位到监听进程（可能是非 TCP 监听或权限受限）".into(),
        });
    }
    // 进程身份校验：仅终止确认是 dsh web 的进程
    let mut confirmed: Vec<u32> = Vec::new();
    let mut unidentified: Vec<u32> = Vec::new();
    for pid in &pids {
        match process_command_line(*pid) {
            Some(cl) if is_dsh_process(&cl) => confirmed.push(*pid),
            _ => unidentified.push(*pid),
        }
    }
    if confirmed.is_empty() {
        let pid_list = pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        return Ok(CleanResult {
            cleaned: false,
            detail: format!("端口 {port} 的占用进程（PID {pid_list}）不是 dsh 服务，已中止清理以避免误杀"),
        });
    }
    for pid in &confirmed {
        kill_tree(*pid);
    }
    std::thread::sleep(std::time::Duration::from_millis(600));
    if probe_port(&cfg) {
        Ok(CleanResult {
            cleaned: false,
            detail: "已结束 dsh 进程但端口仍被占用（可能还有其他进程绑定同一端口）".into(),
        })
    } else {
        let list = confirmed.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        Ok(CleanResult { cleaned: true, detail: format!("已清理残留 dsh 进程: {list}") })
    }
}
