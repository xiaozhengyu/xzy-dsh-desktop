//! 环境检测：PATH 中定位 node / dsh，解析 dsh.cmd shim 得到真实 JS 入口。

use std::{fs, path::PathBuf, process::Command};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// 环境检测结果。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvInfo {
    pub node: Option<String>,
    pub dsh: Option<String>,
    pub dsh_entry: Option<String>,
}

/// 在 PATH 中查找可执行文件，返回第一个带扩展名（.cmd/.exe）的条目。
pub fn where_find(exe: &str) -> Option<String> {
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
pub fn resolve_dsh_entry(shim: &str) -> Option<String> {
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

/// 环境检测：结果缓存于 AppState（force=true 时强制重新探测，用于「重试」按钮）。
#[tauri::command(async)]
pub fn get_env_info(state: State<'_, AppState>, force: Option<bool>) -> EnvInfo {
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
