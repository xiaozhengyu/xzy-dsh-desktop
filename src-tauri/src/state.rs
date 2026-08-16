//! 全局共享应用状态（各命令 / 托盘 / 启动流程共用）。

use std::{
    process::Child,
    sync::{atomic::AtomicBool, Mutex},
    time::SystemTime,
};

use crate::config::AppConfig;
use crate::env::EnvInfo;

/// 全局共享状态。
pub struct AppState {
    /// 本应用派生的 dsh 进程句柄。
    pub child: Mutex<Option<Child>>,
    /// 是否正在退出（托盘「退出应用」置位，窗口关闭不再拦截）。
    pub exiting: AtomicBool,
    /// 环境检测缓存（node/dsh 路径启动后不会变）。
    pub env_info: Mutex<Option<EnvInfo>>,
    /// 配置（运行时可变：set_config 修改后同时持久化到 config.json）。
    pub config: Mutex<AppConfig>,
    /// 本应用派生的 dsh 进程启动时刻（用于前端展示运行时长）。
    pub started_at: Mutex<Option<SystemTime>>,
}
