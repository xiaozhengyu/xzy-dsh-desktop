# AI 变更记录

## 2026-09-18

- 修复新版 `dsh web` 输出带 token 的认证 URL 后，桌面端仍导航到无 token 基础地址导致认证失败的问题。
- 桌面端启动 dsh 时使用 `--no-open`，捕获并传递当前服务的认证 URL，同时保留无 token 输出时的兼容回退。
- 修复自动启动竞态：缩短状态轮询间隔，本应用托管实例允许自动导航，并在实际导航前重新读取认证 URL。
- 修复 WebView 与新版 dsh `SameSite=Strict` 认证 cookie 的跨站问题：使用“dsh 基础页加载完成后再打开 token URL”的两步导航。

## 2026-09-18（个人 Windows 稳定性收口）

- 固定 dsh 服务绑定 `127.0.0.1`，移除设置页的 host 配置，避免个人工具意外向局域网暴露服务。
- 为启动、停止、重启增加串行操作锁；启动超时或进程提前退出时统一终止进程树并清理认证状态。
- 远程 Harness capability 改为匹配 `127.0.0.1` 的配置端口，保留自定义端口时的全屏快捷键权限。
- 声明 `devtools` Cargo feature 并修正可派生的默认配置。

## 2026-09-18（dsh CLI 入口兼容）

- 适配 `dsh 0.1.5-rc.2` 的新入口：使用 `dsh --profile web --no-open --port ...`，避免默认浏览器抢先消费认证 token。
- release 构建不再默认打开 DevTools，保留 debug 或显式 feature 下的调试能力。
