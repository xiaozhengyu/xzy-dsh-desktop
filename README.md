# DeepSeek Harness 桌面端（dsh-desktop）

给 DeepSeek Harness CLI（`dsh`）做的 **Windows 11 专属桌面壳**：Tauri 2（Rust + WebView2），
系统托盘常驻、单例运行、一键启动/停止 `dsh web` 服务，并内嵌 Harness 的 Web UI。
**不内嵌任何 Node.js 运行时**——启动时直接调用系统 PATH 中的全局 `node` 与 `dsh`。

## 硬性要求对照

| 要求 | 实现 |
|---|---|
| 技术栈 | Tauri 2（Rust + Vanilla JS 前端） |
| 内存目标 | 壳本体（主进程）私有内存 ≈ 60–70 MB；整体含 WebView2 见下方「内存说明」 |
| 平台 | 仅 Windows 11（WebView2 内置）；代码不引入任何跨平台分支逻辑 |
| 不内嵌 Node.js | 通过 `where node` / `where dsh` 定位系统全局命令；解析 `dsh.cmd` shim 得到真实 JS 入口后用 `Command::new(node)` 直接执行；解析失败回退 `cmd /C dsh` |
| 安装包 | NSIS 安装包（`tauri build`）或便携版 exe（`tauri build --no-bundle`），exe ≈ 3.3 MB、安装包 ≈ 1.2 MB |

## 功能清单

- **启动服务**：`node <dsh 入口> web --port 3080`，日志写入
  `%LOCALAPPDATA%\com.deepseek.harness-desktop\logs\dsh-web.log`
- **停止服务**：`taskkill /PID <pid> /T /F` 强杀整棵进程树，防止端口残留
- **端口检测**：启动前探测 `127.0.0.1:3080` / `[::1]:3080`；已被占用 → 提示
  “Harness 可能已在运行”，直接加载现有实例
- **内嵌 Web UI**：运行中自动在 iframe 中加载 `http://127.0.0.1:3080`（已验证 dsh 的
  Web 服务不发送 X-Frame-Options / CSP frame-ancestors，可嵌入）
- **系统托盘**：关闭主窗口 → 最小化到托盘；托盘菜单：显示主窗口 / 退出应用（退出时先杀进程树）
- **单例模式**：`tauri-plugin-single-instance`，重复启动自动唤醒已有窗口
- **环境引导**：Node.js / dsh 缺失时界面明确提示，并给出
  `npm install -g @deepseek/dsh` 安装命令；支持「重新检测」无需重启
- **Mica 背景**：`window-vibrancy` 应用 Windows 11 云母材质（失败时自动降级为纯色）

## 目录结构

```
xzy-dsh-desktop/
├── index.html                  # 前端入口（控制面板 + 内嵌 iframe）
├── src/
│   ├── main.js                 # 前端控制逻辑（Vanilla JS）
│   └── styles.css              # Win11 极简暗色样式
├── scripts/
│   ├── frontend.mjs            # 复制前端 → dist/；--serve 启动 dev 静态服务器
│   ├── generate-icon.ps1       # 生成 assets/app-icon.png（1024×1024）
│   └── crates-mirror.mjs       # （仅沙箱构建用）本地 crates.io 代理，正常机器可删除
├── assets/app-icon.png         # 图标源图
└── src-tauri/
    ├── Cargo.toml              # tauri 2 / single-instance / window-vibrancy
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

## 运行说明

- 启动后点击「启动服务」→ Rust 后端以系统 node 启动 `dsh web --port 3080`，
  端口就绪后界面自动内嵌加载 Harness Web UI
- 关闭窗口 = 最小化到托盘（右下角图标）；托盘右键「退出应用」= 真正退出并强杀 dsh 进程
- 若 3080 已被占用（比如 Harness 已在别处运行），顶栏状态点显示为琥珀色并直接加载
  现有实例，不会重复启动

## 内存说明

实测（Windows 11，`WorkingSet64`，含共享内存页面，偏保守）：

- **壳本体（Rust 主进程）**：约 68 MB（私有内存约 30–40 MB）
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

## 已知边界

- 异常强杀（任务管理器结束进程）不会触发退出清理，可能残留 node/dsh 进程；
  再次启动前应用会检测端口占用并直接复用（或手动 `taskkill /IM node.exe /T /F`）
- 中文/空格路径安全：所有命令均以参数数组方式传给 `std::process::Command`，
  不经字符串拼接进 shell，Windows 原生 CreateProcess 引用规则自动处理
