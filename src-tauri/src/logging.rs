//! 日志：dsh-web.log 的增量读取、清空、资源管理器定位、正常退出标记。

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    process::Command,
};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use serde::Serialize;
use tauri::{AppHandle, Manager};

/// 往 dsh-web.log 追加一行（正常退出标记用）。
pub fn append_log_line(app: &AppHandle, line: &str) {
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

/// 日志文件路径（缺失时为空路径）。
fn log_path(app: &AppHandle) -> std::path::PathBuf {
    app.path()
        .app_log_dir()
        .map(|d| d.join("dsh-web.log"))
        .unwrap_or_default()
}

/// 在资源管理器中定位日志文件（explorer /select）。
#[tauri::command]
pub fn open_log_folder(app: AppHandle) -> Result<(), String> {
    let path = log_path(&app);
    let mut cmd = Command::new("explorer");
    cmd.arg("/select,").arg(&path);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    cmd.spawn().map_err(|e| format!("无法打开资源管理器: {e}"))?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailResult {
    pub offset: u64,
    pub lines: Vec<String>,
    pub truncated: bool,
}

/// 增量读取 dsh-web.log：从上次 offset 读到文件尾。
/// 文件被截断（如被清空/轮转）时从头读取并置 truncated。
#[tauri::command]
pub fn tail_log(app: AppHandle, offset: u64) -> TailResult {
    let path = log_path(&app);
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return TailResult { offset: 0, lines: vec![], truncated: false };
    }
    let truncated = offset > size;
    let start = if truncated { 0 } else { offset };
    let mut content = String::new();
    if let Ok(mut f) = File::open(&path) {
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
pub fn clear_log(app: AppHandle) -> Result<(), String> {
    let path = log_path(&app);
    fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| format!("无法清空日志: {e}"))?;
    Ok(())
}
