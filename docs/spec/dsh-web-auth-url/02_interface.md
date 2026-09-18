# dsh Web 认证 URL 接口契约

## Rust 共享状态

`AppState.authenticated_url` 保存当前由本应用托管的 dsh 进程输出的认证 URL：

- 服务启动前、停止后、进程异常退出后为 `None`；
- 解析到 `dsh web: http://...?token=...` 后设置为该 URL；
- 不写入配置文件，不跨进程持久化。

## `get_status`

在现有响应字段基础上增加：

```json
{
  "authenticatedUrl": "http://127.0.0.1:3081/?token=..."
}
```

没有认证 URL 时返回 `null`。现有 `portInUse`、`owned` 和 `url` 字段保持兼容。

## `start_service`

现有 `Started` / `AlreadyRunning` 响应中的 `url` 改为有效导航 URL：

- 优先使用当前进程捕获的认证 URL；
- 否则回退到配置生成的基础 URL。

## 前端状态

- `baseWebUrl` 保存配置生成的基础 URL；
- `webUrl` 保存当前用于导航和展示的有效 URL；
- `get_status.authenticatedUrl` 存在时覆盖 `webUrl`，否则使用 `baseWebUrl`。

新增 `navigate_to_harness` 命令。它先导航到 dsh 基础页，等待 Tauri 的 `on_page_load(Finished)` 回调，再导航到带 token 的 URL，使 token 换取 cookie 的请求从 dsh 自身 origin 发起，满足新版 dsh 的 `SameSite=Strict` 认证约束。

## dsh 启动参数

桌面端通过现有 `dsh web` 别名启动，并追加：

```text
--no-open --port <port>
```

`--no-open` 防止 dsh 启动系统默认浏览器；认证 URL 仍从 stdout/stderr 的 `dsh web:` 行获取。
