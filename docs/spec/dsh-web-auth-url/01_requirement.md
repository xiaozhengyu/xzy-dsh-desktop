# dsh Web 认证 URL 兼容性需求

## 背景

新版全局 `dsh` 启动 Web 服务后会输出带一次性认证 token 的 URL，例如
`http://127.0.0.1:3081/?token=...`。桌面端目前只导航到不带 token 的固定地址，导致内嵌 WebView 显示认证失败。

## 目标

桌面端启动自身托管的 `dsh web` 服务后，必须使用 dsh 输出的认证 URL 打开 Harness Web UI，同时保留旧版 dsh 不输出认证 URL 时的基础地址回退行为。

## 范围

### In Scope

- 启动 dsh 时禁止它额外打开系统默认浏览器。
- 捕获 dsh 输出中的认证 URL，并在 Rust 状态、Tauri 命令响应和前端导航之间传递。
- 服务停止、重启或异常退出时清理失效的认证 URL。
- 保持现有基础 URL 配置和 Tauri 远程 origin 权限兼容。
- 增加 URL 解析测试，并更新运行说明和变更记录。

### Out of Scope

- 修改 dsh 本身的认证机制。
- 持久化或跨应用重启复用 token。
- 为外部已运行的 dsh 实例推断其未由本应用捕获的 token。

## 验收标准

1. 使用当前新版 dsh 启动服务时，应用不再额外打开系统浏览器。
2. 启动成功后的“进入 Harness”和自动导航使用带 `token` 查询参数的 URL。
3. 控制台状态和服务地址显示当前有效 URL；停止服务后回到基础 URL。
4. 无认证 URL 输出的旧版 dsh 仍可按基础 URL 工作。
5. Rust 测试、前端构建和 Tauri 构建通过。
