# 控制台增强 + 独立设置页 — 设计文档

日期：2026-08-16 · 状态：待用户审阅 · 版本：0.1

## 1. 背景与目标

当前「控制台」页只有一条顶栏（图标 + 状态 + 启停 + 窗口控制）和一整块空白主区，
内容过于单薄。本设计将控制台改造成**整页仪表盘**（服务信息 + 运维工具 + 日志查看），
并新增**独立设置页**承载全部配置项。所有功能按性价比分 P0–P3 四批实现，每批可独立验证。

关键前置事实（已核实）：

- 主窗口 `decorations(true)`：系统原生标题栏（含原生最小化/最大化/关闭）一直存在，
  控制台页自定义窗口按钮是冗余的
- 托盘已有「控制台」菜单项，且已实现 `navigate("http://tauri.localhost/")` 返回本地页
- 全屏快捷键（F11/Esc）由 `initialization_script` 注入，本地页与 Harness 页均生效
- 前端构建为纯拷贝（`scripts/frontend.mjs` 递归拷贝 `src/`），多 JS 文件可直接用 `<script>` 标签

## 2. 范围

| 批次 | 内容 |
|---|---|
| P0 | 顶栏移除、服务/环境信息面板、日志查看器（tail + 高亮 + 过滤 + 清空） |
| P1 | 一键重启、自检诊断、检查 dsh 更新 |
| P2 | 独立设置页（端口/主机、开机自启、启动时自动启动服务、深浅主题、DevTools、关于）、托盘「设置」菜单项 |
| P3 | 异常强杀检测提示、一键清理残留进程 |

**非目标（YAGNI）**：Harness 页内悬浮工具栏（方案 C，二期再说）；日志远程查看/下载；
多服务实例管理；npm 自动安装 dsh；更换应用图标。

## 3. 架构决策

### 3.1 顶栏移除，功能并入页面

- 删除 `index.html` 中的 `#fs-chrome` / `.topbar` / 自定义窗口按钮、`styles.css` 中对应样式
  与 `#app.fs` 全屏滑出逻辑、`main.js` 中 `initControls()` 与 `updateFsUi()`
- 状态点/文字与启停按钮并入**页面头部卡片**（控制台视图顶部）
- 窗口拖拽、最小化/最大化/关闭 → 由系统原生标题栏承担；全屏进出 → 仅 F11/Esc + 托盘
- 关闭按钮行为不变（`CloseRequested` → 隐藏到托盘）

### 3.2 单页 + hash 视图切换（控制台 / 设置）

- 同一个 `index.html`，两个视图：`#/` 控制台（默认）、`#/settings` 设置
- 前端监听 `hashchange` + 启动时读 `location.hash` 决定初始视图
- **视图切换不重载页面**：轮询定时器、`state`、日志读取偏移全部保留在同一个 JS 上下文
- 控制台视图头部右侧加「⚙ 设置」入口；设置页顶部加「← 返回控制台」，均为改 hash，不重载
- 托盘导航目标：`http://tauri.localhost/#/`（控制台）与 `http://tauri.localhost/#/settings`（设置）
- 轮询调度：`get_status` 3s 轮询保持常驻；日志 tail 1.5s 仅在控制台视图可见时运行
  （视图隐藏时暂停，复用现有 `document.hidden` 模式）

### 3.3 托盘菜单（新增「设置」项）

```
控制台        ← 已有；navigate 目标补为 http://tauri.localhost/#/
设置          ← 新增：show + unminimize + navigate(http://tauri.localhost/#/settings)
────────────
全屏 / 退出全屏 ← 已有
退出应用       ← 已有
```

实现与现有 `"back"` 分支同构（`build_tray` 增加 `MenuItem` id `"settings"`）。

### 3.4 前端模块拆分（保持单元小而清晰）

`main.js` 当前 317 行，加入两个视图后会明显膨胀，拆为普通 `<script>` 标签多文件
（`frontend.mjs` 递归拷贝，无需构建器）：

| 文件 | 职责 |
|---|---|
| `src/ipc.js` | `invoke` 封装与命令名常量 |
| `src/main.js` | 入口：hash 路由、初始化、轮询调度、视图切换 |
| `src/views-console.js` | 控制台视图：头部卡片、服务/环境卡片、操作行、日志查看器 |
| `src/views-settings.js` | 设置视图：各设置卡片、保存/应用逻辑、关于 |
| `src/styles.css` | 统一样式 + 浅色主题变量 |

### 3.5 配置模型（`config.json` 扩展）

```json
{
  "web": { "host": "127.0.0.1", "port": 3081 },
  "service": { "startTimeoutSecs": 25, "autoStart": true },
  "devtools": { "autoOpen": false },
  "theme": { "mode": "system" },
  "autostart": false
}
```

- 新增字段 `service.autoStart`（Rust 已有该逻辑，只缺 UI）、`theme.mode`（"system"|"light"|"dark"）、`autostart`（注册表自启，属壳层状态，写入配置便于 UI 显示）
- 端口/主机保存后**下次启动服务时生效**（dsh web 按旧端口运行中，需停止→启动），保存后 UI 明确提示

## 4. Rust 命令清单

全部走 `capabilities/default.json` 声明 allow 权限（README 踩坑 #1 的老规矩，均只在本地页调用）；
所有外部进程调用加 `CREATE_NO_WINDOW`（踩坑 #15）。

| 命令 | 入参 | 返回 | 说明 |
|---|---|---|---|
| `get_service_info` | — | `{ pid, startedAt, logPath }` | PID、启动时刻、日志路径；AppState 增存 `started_at` |
| `open_log_folder` | — | `()` | `explorer /select,<log>` |
| `tail_log` | `offset: u64` | `{ offset, lines, truncated }` | 从偏移增量读日志；文件被截断（size<offset）时重置从头读并置 `truncated` |
| `clear_log` | — | `()` | 截断日志文件 |
| `restart_service` | — | 同 `start_service` | stop→start；**外部占用（非本应用托管）时拒绝**（错误码 `NotOwned`） |
| `run_diagnostics` | — | `[{ check, ok, detail }]` | node/dsh 存在与版本、dsh 入口解析、端口占用归属、日志目录可写、配置文件可读、web 连通性 |
| `check_update` | — | `{ current, latest, hasUpdate } \| null` | `npm view @deepseek/dsh version`（async，超时 8s，失败返回 null 不报错）；结果缓存 10 分钟 |
| `get_config`（扩展） | — | 增加 `autoStart` / `themeMode` / `autostart` | 前端 `applyConfig` 读取 |
| `set_config` | `{ webPort?, webHost?, autoStart?, themeMode?, autoOpenDevtools? }` | `()` | 写回 `config.json`（serde_json 已有依赖） |
| `set_autostart` | `enabled: bool` | `()` | `HKCU\...\CurrentVersion\Run` 键增删（值为当前 exe 路径），并同步写入 `config.json` 的 `autostart` 镜像（供 UI 开关显示） |
| `apply_theme` | `dark: bool` | `()` | 重应用 Mica（`window_vibrancy` 深浅参数） |
| `clean_stale` | — | `{ cleaned: bool, detail }` | `netstat -ano` 找占用端口 PID → `taskkill /PID /T /F` |

> `write_shutdown_mark` 为 Rust 内部 helper（非 Tauri 命令、不走 ACL）：`stop_service` 成功、
> 应用正常退出（`RunEvent::ExitRequested` 前）时调用，在日志追加 `--- normal shutdown ---`。
任务管理器强杀不会触发 → 日志尾部无标记即视为异常退出。

## 5. 控制台视图（P0 布局）

```
┌─ 系统原生标题栏 ────────────────────────────────────────┐
├──────────────────────────────────────────────────────┤
│ 页面头部：🐋 DSH 桌面端  ● 服务运行中   [启动服务/停止]  ⚙ │
├──────────────────────────────────────────────────────┤
│ 服务卡片：端口 3081 · http://127.0.0.1:3081 [复制]      │
│           PID 12345 · 已运行 2 小时 3 分 · 日志 [打开]  │
├──────────────────────────────────────────────────────┤
│ 环境卡片：node v22.14.0 (路径) · dsh 0.5.0 (路径)      │
│           [重新检测]（缺失时红色 banner + 安装命令提示） │
├──────────────────────────────────────────────────────┤
│ 操作行：[进入 Harness] [重启服务] [自检诊断] [检查更新]  │
├──────────────────────────────────────────────────────┤
│ 日志查看器：过滤框 [清空] · monospace 输出区             │
│           自动滚动 + 回到底部徽章 · 错误行高亮          │
└──────────────────────────────────────────────────────┘
```

- 服务卡片「复制」用 `navigator.clipboard.writeText`（`tauri.localhost` 为 secure context），
  失败回退 `document.execCommand("copy")`
- 运行时长由 `startedAt` 计算，随 3s 轮询刷新
- 操作行按钮按批次点亮：P0 先有「进入 Harness」「打开日志文件夹」；P1 增加「重启服务」「自检诊断」「检查更新」
- 日志查看器：错误行高亮（`error/fail/panic/exception/异常` 关键词）；滚动条在底部时跟随新行，
  用户上翻停止跟随并显示「回到底部」徽章；关键字过滤输入框；「清空」需确认
- 服务未启动时：服务卡片显示「未运行」，环境卡片照常显示，日志查看器显示历史日志（若有）

## 6. 设置视图（P2）

| 卡片 | 设置项 | 实现 |
|---|---|---|
| 服务 | 端口、主机 | 表单 → `set_config`；提示「下次启动服务时生效」 |
| 启动 | 开机自启、启动时自动启动服务 | `set_autostart` + `set_config(autoStart)` 开关 |
| 外观 | 主题三选（浅色/深色/跟随系统） | radio → `set_config(themeMode)` + `apply_theme` + 前端 `body` class 切 CSS 变量；跟随系统用 `prefers-color-scheme` 实时响应 |
| 调试 | DevTools 自动打开 | `set_config(devtools.autoOpen)` 开关（已有字段，补 UI） |
| 关于 | 应用版本、图标署名（CC BY-NC-SA 4.0）、配置文件路径、GitHub 链接 | 纯展示 |

> 两个「启动」概念的区别，UI 文案须明确：**开机自启** = Windows 登录时自动启动本应用（注册表）；
> **启动时自动启动服务** = 本应用启动后自动执行「启动服务」（`service.autoStart`，Rust 已有逻辑）。

顶部「← 返回控制台」；读取当前值：`get_config`（扩展后）+ `get_env_info`。

## 7. 异常兜底（P3）

- 控制台视图加载时：读取日志尾部（最近 50 行），若日志非空、服务未运行、且**无**
  `--- normal shutdown ---` 标记 → 顶部琥珀色提示「上次可能异常退出」
- 提示附带「一键清理残留进程」按钮 → `clean_stale`（复用 README 踩坑 #16 的思路自动化）
- 无残留则提示「未发现残留进程」

## 8. 错误处理与测试

### 错误处理

- 所有新命令返回结构化结果，前端一律**不阻塞 UI**；失败在卡片内/横幅提示
- `check_update` 离线/无 npm 时返回 null，静默降级，不打扰
- `restart_service` 在外部占用时返回 `NotOwned`，前端禁用该按钮（与启停按钮同一判定）

### 测试与验证

手测清单：

1. 启停/重启：正常路径 + 外部占用时禁用/拒绝
2. 日志：增量显示、清空、关键字过滤、错误行高亮、上翻后停止跟随 + 回到底部
3. 自检：正常环境全绿；杀进程后能看到残留提示
4. 检查更新：在线（有/无新版）、离线（静默降级）
5. 设置：端口/主机保存、自启注册表键增删、主题切换（含 Mica 深浅）、DevTools 开关
6. 托盘：新「设置」项在本地页与 Harness 页均可直达设置视图；「控制台」项回归
7. 全屏回归：F11/Esc 在控制台页与 Harness 页仍生效；顶栏滑出逻辑已删，无残留引用
8. 关闭到托盘：原生关闭按钮行为不变

构建注意事项（沿用 README 备忘）：必须经 Tauri CLI 构建；exe 被占用时先杀进程再构建；
`Cargo.toml` 行尾符噪音提交前还原。

## 9. 风险与已知边界

- **clipboard**：`navigator.clipboard` 在 WebView2 的 `tauri.localhost` 视为 secure context，
  已备 execCommand 回退
- **Mica 深浅切换**：`window_vibrancy::apply_mica` 切换是运行时可调，若个别版本不稳定则降级为
  仅前端 CSS 变量切换（非致命）
- **端口归属判定**依赖 `netstat -ano` 解析，进程名/权限异常时诊断为「未知归属」，不误杀
- **自动启动**只保证当前 exe 路径；便携版移动位置后自启失效属预期
- 设置变更（端口等）不热生效，需重启服务——已通过 UI 文案明确

## 10. 实施顺序

1. P0：Rust（`get_service_info`/`open_log_folder`/`tail_log`/`clear_log` + ACL）→ 前端拆分与
   控制台视图 → 顶栏移除
2. P1：`restart_service`/`run_diagnostics`/`check_update` + 操作行按钮
3. P2：`set_config`/`set_autostart`/`apply_theme` + 设置视图 + 托盘「设置」项
4. P3：正常退出标记 + 异常检测 + `clean_stale`
