# AI 变更记录

## 2026-09-18

- 修复新版 `dsh web` 输出带 token 的认证 URL 后，桌面端仍导航到无 token 基础地址导致认证失败的问题。
- 桌面端启动 dsh 时使用 `--no-open`，捕获并传递当前服务的认证 URL，同时保留无 token 输出时的兼容回退。
- 修复自动启动竞态：缩短状态轮询间隔，本应用托管实例允许自动导航，并在实际导航前重新读取认证 URL。
- 修复 WebView 与新版 dsh `SameSite=Strict` 认证 cookie 的跨站问题：使用“dsh 基础页加载完成后再打开 token URL”的两步导航。
