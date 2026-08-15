# DeepSeek Harness 桌面端（dsh-desktop）

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
- **整窗进入 Harness**：服务就绪后自动整窗导航到 `http://127.0.0.1:3081`（跨站 iframe 不可行，
  改为顶层导航）
- **系统托盘**：关闭主窗口 → 最小化到托盘；托盘菜单：显示主窗口 / 返回控制台 / 启动·停止服务
  （同一项，随服务状态切换文案）/ 全屏 / 退出应用（退出时先杀进程树）
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
    ├── tauri.conf.json         # 窗口、托盘、NSIS 打包配置
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
- 关闭窗口 = 最小化到托盘（右下角图标）；托盘菜单：显示主窗口 / 返回控制台 / 启动·停止服务
  （同一项，随服务状态切换文案）/ 全屏 / 退出应用；「退出应用」= 真正退出并强杀 dsh 进程
- 若 3081 已被占用（比如 Harness 已在别处运行），顶栏状态点显示为琥珀色并直接加载
  现有实例，不会重复启动
- 托盘菜单「全屏 / 退出全屏」：一键进入/退出全屏；全屏时顶栏自动隐藏，鼠标移到屏幕
  顶部边缘即滑出

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
