# DSH 桌面端（dsh-desktop）

给 DeepSeek Harness CLI（`dsh`）做的 **Windows 11 专属桌面壳**：Tauri 2（Rust + WebView2），
系统托盘常驻、单例运行、一键启动/停止 `dsh web` 服务，并内嵌 Harness 的 Web UI。
界面为极简单行顶栏（状态 + 启停 + 窗口控制）+ 全屏 Harness Web 界面。
**不内嵌任何 Node.js 运行时**——启动时直接调用系统 PATH 中的全局 `node` 与 `dsh`。

## 硬性要求对照

| 要求 | 实现 |
|---|---|
| 技术栈 | Tauri 2（Rust + Vanilla JS 前端） |
| 内存目标 | 壳本体（主进程）私有内存 ≈ 41 MB；整体含 WebView2 见下方「内存说明」 |
| 平台 | 仅 Windows 11（WebView2 内置）；代码不引入任何跨平台分支逻辑 |
| 不内嵌 Node.js | 通过 `where node` / `where dsh` 定位系统全局命令；解析 `dsh.cmd` shim 得到真实 JS 入口后用 `Command::new(node)` 直接执行；解析失败回退 `cmd /C dsh` |
| 安装包 | NSIS 安装包（`tauri build`）或便携版 exe（`tauri build --no-bundle`），exe ≈ 3.3 MB、安装包 ≈ 1.2 MB |

## 功能清单

- **启动服务**：`node <dsh 入口> web --port 3081`，日志写入
  `%LOCALAPPDATA%\com.deepseek.harness-desktop\logs\dsh-web.log`
- **停止服务**：`taskkill /PID <pid> /T /F` 强杀整棵进程树，防止端口残留
- **端口检测**：启动前探测 `127.0.0.1:3081` / `[::1]:3081`；已被占用时顶栏显示琥珀色
  状态点并直接加载现有实例，不会重复启动
- **全屏**：托盘菜单「全屏 / 退出全屏」一键切换（文案随当前状态自动变化）；全屏时顶栏自动隐藏，
  鼠标移到屏幕顶部边缘即滑出
- **全屏快捷键**：`F11` 切换全屏、`Esc` 退出全屏；快捷键在应用控制台页与 Harness 界面均可用
  （解决主屏全屏时任务栏/托盘被盖住、无法退出的问题）；Esc 优先让给 Harness 自身使用，
  仅当页面未拦截且焦点不在输入控件内时退出全屏
- **整窗进入 Harness**：服务就绪后自动整窗导航到 `http://127.0.0.1:3081`（跨站 iframe 不可行，
  改为顶层导航）
- **系统托盘**：关闭主窗口 → 最小化到托盘；托盘菜单：返回控制台 / 全屏 / 退出应用
  （退出时先杀进程树）；服务启停由控制台页顶栏按钮负责，托盘不做后台轮询
- **单例模式**：`tauri-plugin-single-instance`，重复启动自动唤醒已有窗口
- **环境引导**：Node.js / dsh 缺失时界面明确提示，并给出
  `npm install -g @deepseek/dsh` 安装命令；支持一键「重试」无需重启应用
- **Mica 背景**：`window-vibrancy` 应用 Windows 11 云母材质（失败时自动降级为纯色）

## 目录结构

```
xzy-dsh-desktop/
├── index.html                  # 前端入口（极简顶栏 + 整窗 Harness）
├── src/
│   ├── main.js                 # 前端控制逻辑（Vanilla JS）
│   ├── styles.css              # Win11 极简暗色样式
│   └── brand-icon.png          # 顶栏鲸鱼娘图标（64×64）
├── scripts/
│   ├── frontend.mjs            # 复制前端 → dist/；--serve 启动 dev 静态服务器
│   ├── generate-icon.ps1       # 生成 assets/app-icon.png（1024×1024）
│   ├── fetch-icon.mjs          # 下载鲸鱼娘图标原始素材到 assets/
│   └── crates-mirror.mjs       # （仅沙箱构建用）本地 crates.io 代理，正常机器可删除
├── assets/app-icon.png         # 图标源图
└── src-tauri/
    ├── Cargo.toml              # tauri 2 / single-instance / window-vibrancy
    ├── build.rs                # 构建脚本（icons 变更自动触发资源重嵌入）
    ├── tauri.conf.json         # NSIS 打包、安全与全局配置（主窗口在 main.rs 中创建）
    ├── capabilities/default.json
    └── src/main.rs             # 全部 Rust 后端逻辑
```

## 编译运行

前置条件：

1. **Rust 工具链**（MSVC）：`rustup` 安装 stable + `x86_64-pc-windows-msvc`，
   以及 Visual Studio Build Tools（含 C++ 桌面开发组件，本项目已用 VS 18 + MSVC 14.50 验证）
2. **Node.js**（≥ 18，本项目在 v22 验证）——用于前端静态资源与 Tauri CLI
3. **全局 dsh**：`npm install -g @deepseek/dsh`
4. **Windows 11**（自带 WebView2 Runtime）

构建：

```powershell
# 1. 安装 Tauri CLI（前端零依赖，无需其他 npm 包）
npm install

# 2.（可选）重新生成图标：assets/app-icon.png → src-tauri/icons/
npm run icons

# 3a. 开发模式（热更新前端，需保持 dev 服务器运行）
npm run tauri dev

# 3b. 打包 NSIS 安装包（输出到 src-tauri/target/release/bundle/nsis/）
npm run build

# 3c. 仅生成便携版 exe（不打包安装程序）
npm run build:portable
```

首次 `cargo` 编译需下载依赖（tauri 全链路约 5–15 分钟），之后增量编译很快。

> ⚠️ **必须用 Tauri CLI 编译（`npm run build` / `npm run build:portable`），CLI 会自动启用
> `tauri/custom-protocol` feature**。若跳过 CLI 直接用 `cargo build --release`（不带该
> feature），会因 `tauri.conf.json` 配置了 `devUrl` 而按 **dev 模式** 构建，**前端资源不会
> 嵌入 exe**——运行后应用页面无法加载，显示 `asset not found: index.html` 或「无法访问此页面」。
> 确需直接跑 cargo 时，请手动补上 feature：
>
> ```powershell
> cargo build --release --features tauri/custom-protocol
> ```

## 运行说明

- 启动后点击「启动服务」→ Rust 后端以系统 node 启动 `dsh web --port 3081`，
  端口就绪后界面自动整窗进入 Harness Web UI
- 关闭窗口 = 最小化到托盘（右下角图标）；托盘菜单：返回控制台 / 全屏 / 退出应用；
  「退出应用」= 真正退出并强杀 dsh 进程；服务启停、状态查看均在控制台页完成
- 若 3081 已被占用（比如 Harness 已在别处运行），顶栏状态点显示为琥珀色并直接加载
  现有实例，不会重复启动
- 托盘菜单「全屏 / 退出全屏」：一键进入/退出全屏；全屏时顶栏自动隐藏，鼠标移到屏幕
  顶部边缘即滑出
- 快捷键：`F11` 切换全屏、`Esc` 退出全屏（Esc 在输入框内不生效，优先让 Harness 使用）
  —— 主屏全屏时任务栏/托盘不可达，用 `F11`/`Esc` 即可进出

## 内存说明

实测（Windows 11，工作集口径含共享内存页面，偏保守）：

- **壳本体（Rust 主进程）**：工作集约 68 MB，私有内存约 41 MB
- **WebView2 基础进程**（browser/GPU/network/utility，WebView2 体系固有成本）：约 300 MB 工作集
- **内嵌 Harness UI 的渲染进程**：约 130–200 MB（Harness Web UI 本身是重型 React 应用）

因此整棵进程树约 450–600 MB 工作集（服务运行、自动内嵌 Harness UI 时）。150 MB 目标在
“WebView2 + 内嵌完整 Harness UI”前提下无法达成——这是 WebView2 多进程体系 + 重型前端
决定的，Tauri 已是同场景最轻方案（Electron 同类场景通常 700 MB+）。

dsh 服务本身是独立 Node 进程，另计约 60–100 MB（不含在壳内）。

## 图标署名

应用图标使用 [deepseek-whale-girl-icon](https://github.com/fornarwhal/deepseek-whale-girl-icon) 中的
`improved-1.png`（984×984，透明底），授权协议 **CC BY-NC-SA 4.0（须署名、非商用、相同方式共享）**：

- 角色形象来源：上善无形（原创 OC「溟月」）
- DeepSeek 元素二创：ZipZipPipe（GPT Image 2）
- 改进版修复：QYQCAMIAO

> ⚠️ 非商用许可：若本应用用于商业用途，请更换图标或另行取得授权。

重新生成图标：`npm run icons` 后执行 `npm run build` 即可（`build.rs` 已声明
`rerun-if-changed=icons`，换图标会自动重新嵌入 exe 资源，无需手动清理）。

## 配置文件

首次运行会在 `%APPDATA%\com.deepseek.harness-desktop\config.json` 自动生成配置文件，
修改后**重启应用生效**：

```json
{
  "web": { "host": "127.0.0.1", "port": 3081 },
  "service": { "startTimeoutSecs": 25 },
  "devtools": { "autoOpen": false }
}
```

- `web.port`：dsh web 服务端口（默认 3081，避开 Harness 默认 3080，可与现有会话并存）
- `web.host`：服务绑定主机（默认回环地址）
- `service.startTimeoutSecs`：服务启动等待上限（秒）
- `devtools.autoOpen`：调试用，启动时自动打开开发者工具

## 已知边界

- 异常强杀（任务管理器结束进程）不会触发退出清理，可能残留 node/dsh 进程；
  再次启动前应用会检测端口占用并直接复用（或手动 `taskkill /IM node.exe /T /F`）
- 中文/空格路径安全：所有命令均以参数数组方式传给 `std::process::Command`，
  不经字符串拼接进 shell，Windows 原生 CreateProcess 引用规则自动处理

## 开发踩坑记录（Tauri 2 备忘）

> 一次「F11/Esc 全屏快捷键在 Harness 页失效」问题的完整排查沉淀。教训很多，写下来避免重蹈覆辙。

### 1. Tauri 2 权限系统（ACL / capabilities）

1. **远程页调用任何命令都必须显式配置 capability**。应用整窗导航到 Harness
   （`http://127.0.0.1:3081`）后属于**远程 origin**，所有 IPC（包括核心窗口命令
   `plugin:window|*`）都会被 ACL 拦截。capability 必须同时具备两个约束，**缺一不可**：
   - `remote.urls`：origin 约束（URLPattern glob，如 `http://127.0.0.1:3081/*`）
   - `windows` / `webviews`：目标窗口约束（如 `["main"]`）

   只写 `remote` 不写 `windows` 时，命令虽解析到了远程 origin 却匹配不到任何窗口 →
   被拒。release 构建的错误信息只有笼统的 `Command ... not allowed by ACL`（详细原因
   仅在 debug 构建可见），排查时极易误判为「脚本没运行」。

2. **capability 的 `remote` 字段是单个对象**（`"remote": { "urls": [...] }`），不是数组
   `[{...}]`——写错会直接构建失败（`expected a sequence`）。

3. **`"default"` 不是可用的 app 命令权限标识符**。`permissions: ["default"]` 会报
   `Permission default not found`（除非在 `src-tauri/permissions/` 自行定义权限文件）。
   更简单的做法：**让前端脚本走核心命令**（如 `core:window:allow-set-fullscreen`），
   不要为了快捷键造自定义命令。

4. **核心窗口命令在本地页同样走 ACL**。`default.json` 若只有 `allow-is-fullscreen`
   没有 `allow-set-fullscreen`，本地页的 `setFullscreen()` 也会被静默拒绝（Promise
   reject 被 `.catch` 吞掉，表现为「按了没反应」）。

### 2. WebView2 / Tauri 行为

5. **WebView2 没有浏览器自带的 F11/Esc 全屏行为**。它是嵌入式控件，没有浏览器 UI；
   F11 只是普通 keydown 事件，必须自己注入脚本 + `getCurrentWindow().setFullscreen()`
   模拟。在 Edge 里 F11 有效，是因为那是浏览器自身的窗口行为，与页面无关。

6. **`window.__TAURI__` 在远程页同样存在**。withGlobalTauri 的初始化脚本在每次文档
   创建时注入（含整窗导航后的远程页）；`initialization_script` 挂的脚本同理。不要
   误以为远程页没有 Tauri API 就放弃这条路。

7. **`on_page_load` / `initialization_script` 是 builder 方法**（`mut self -> Self`），
   已构建的 `WebviewWindow` 没有运行时注册入口。需要挂脚本 → 必须用
   `WebviewWindowBuilder` 创建窗口（窗口配置相应地从 `tauri.conf.json` 移到 Rust）。

### 3. 调试与自动化测试方法论

8. **不要用「页面 emit 事件」诊断远程页**。emit 本身也受 ACL 限制，在远程页会静默
   失败——看起来像「脚本没运行」，实际是 emit 被拒，会严重误导排查方向。正确姿势：
   **Rust 侧主动探测**——`eval_with_callback` 轮询页面状态（`location.href` /
   `!!window.__TAURI__` / 脚本安装标记 / 捕获 keydown 到全局变量），辅以 `win.url()`
   记录导航，完全不依赖页面任何通道。

9. **自动化测试发按键前必须先点击窗口**。`AppActivate` 只把窗口带到前台，
   **webview 内部没有键盘焦点**时 keydown 到不了页面。先用 `SetCursorPos` +
   `mouse_event` 点击窗口中心再 `SendKeys`。（详见 `scripts/smoke-test-shortcuts.ps1`）

10. **`MainWindowHandle` 读取前必须 `$p.Refresh()`**，且启动初期可能拿到 14×14 的
    占位窗口句柄；等窗口尺寸稳定后再操作。

11. **构建失败 `拒绝访问 (os error 5)` = exe 被运行中的实例占用**。先
    `Get-Process dsh-desktop | Stop-Process -Force` 再重新构建。

12. **端口 3081 有残留服务时应用不会自动导航**（`runningAtLoad` 逻辑：启动时端口已
    占用则不自动进 Harness）。自动化测试前务必清掉 3081 的监听进程，否则一直停在
    控制台页。

13. **Rust 侧 `app.listen` 需要 `use tauri::Listener`**；`event.payload()` 直接返回
    `&str`（不是 `Option`）。

### 4. 其他

14. **`Cargo.toml` 行尾符会被构建工具改动**（LF/CRLF）。内容零差异但 git 显示已修改；
    提交前 `git checkout -- src-tauri/Cargo.toml` 还原，避免把行尾噪音混进提交。

15. **GUI 应用 spawn 控制台子系统进程会弹终端窗口**。`where`/`taskkill`/`node`/`cmd`
    都是控制台进程，父进程无控制台时 Windows 会为它们新建终端（默认终端是
    Windows Terminal 时就会弹窗）。**所有 `Command::new` 都要加**
    `#[cfg(windows)] cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW`，
    只给 dsh 服务那处加是不够的。

16. **`dsh web`（端口 3081）是独立 node 进程**，异常强杀应用不会自动清理它。
    清理方法：`Get-NetTCPConnection -LocalPort 3081` 找到 `OwningProcess` 后
    `taskkill /PID <pid> /T /F`。
