# dsh Web 认证 URL 实施方案

## 变更模块

- `src-tauri/src/state.rs`：增加内存中的认证 URL。
- `src-tauri/src/main.rs`：初始化新增状态字段。
- `src-tauri/src/service.rs`：管道读取 dsh 输出、解析认证 URL、返回有效 URL并清理生命周期状态。
- `src/main.js`：同步认证 URL，导航和配置刷新使用正确的基础/有效 URL分层。
- `src-tauri/capabilities/harness-remote.json`：允许本机配置端口的 Harness 页面使用全屏快捷键。
- `README.md`：更新 dsh 启动与认证行为说明。
- `docs/decisions/AI_CHANGELOG.md`：记录本次兼容性修复。

## 核心流程

1. Rust 使用 `Stdio::piped()` 启动 `dsh --profile web --no-open --port ...`。
2. stdout 与 stderr 各由后台线程按行读取；原始行追加到现有日志，并从 `dsh web:` 行提取带 token 的 URL。
3. 服务健康检查成功后短暂等待认证 URL；拿到则返回认证 URL，超时则使用基础 URL兼容旧版本。
4. 前端调用 `navigate_to_harness`；Rust 先导航到 dsh 基础页，再由 Tauri `on_page_load(Finished)` 回调导航到认证 URL，使新版 dsh 的 `SameSite=Strict` cookie 在同站点上下文中建立。自动启动的本应用实例即使在首次轮询前已就绪也允许自动导航。
5. 停止、重启和发现子进程退出时清空认证 URL。
6. 启动、停止和重启通过共享操作锁串行执行；启动超时或提前退出时终止子进程树并回滚状态。
7. host 固定为 `127.0.0.1`，仅保留端口配置，避免个人工具意外暴露到局域网。

## 异常边界

- dsh 不输出符合格式的认证 URL时不阻断服务启动，回退基础 URL。
- dsh 进程提前退出时保留现有 `NotReady` 错误行为。
- 外部占用端口没有本应用捕获的 token，继续使用基础 URL并保持现有外部实例语义。

## 验证

- `cargo fmt --check`
- `cargo test`
- `npm run build:front`
- `npm run build:portable`（环境允许时）
- 使用已安装的 `dsh 0.1.5-rc.2` 验证输出 URL 被捕获且不触发默认浏览器。
