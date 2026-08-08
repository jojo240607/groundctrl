# GroundControl — Tauri (Web) 桌面端

`groundctrl-tauri` 是基于 **Tauri v2** 的桌面地面站，后端为 Rust（复用 `groundctrl-core`），
前端为原生 HTML/CSS/JS + Canvas（`ui-web/`，无运行时框架，经 Vite 打包）。

## 工程结构

```
groundctrl-tauri/        后端 (Rust + Tauri v2)
  src/lib.rs             run()：Tauri Builder + 命令注册 + 订阅启动
  src/commands.rs        Tauri 命令 (connect/disconnect/get_fleet/params/mission/settings...)
  src/subscribe.rs       后台任务：周期推送 fleet 快照 + 转发 BusEvent
  src/state.rs           应用状态 (TelemetryHub + 设置文件)
  src/frontend.rs        前端专用序列化类型 (Snapshot/Alarm/Waypoint/Settings...)
  tauri.conf.json        窗口/构建/图标配置
  capabilities/          前端权限 (命令调用 + 事件监听)
  icons/                 应用图标
ui-web/                  前端 (Vite, 零运行时框架)
  index.html / style.css 布局与深色主题
  main.js                状态管理 + Tauri invoke/事件封装
  map.js / attitude.js / gauges.js / trend.js   Canvas 绘制
```

## 前后端契约

### 命令 (invoke)
| 命令 | 参数 | 返回 |
|------|------|------|
| `connect` | `{kind:"sim"\|"udp"\|"serial", bind, target}` | `void` |
| `disconnect` | — | `void` |
| `get_fleet` | — | `FleetSnapshot` |
| `get_settings` / `save_settings` | `SettingsJson` | `SettingsJson` / `void` |
| `get_monitor_config` / `set_monitor_config` | `MonitorConfigJson` | `MonitorConfigJson` / `void` |
| `request_params` | `{sys, comp}` | `void` |
| `set_param` | `{sys, comp, name, value}` | `void` |
| `upload_mission` | `{sys, comp, items:[WaypointItem]}` | `void` |

### 事件 (listen)
| 事件 | 载荷 |
|------|------|
| `fleet` | `FleetSnapshot { vehicles:[VehicleSnapshot], activeLink, time }` |
| `alarm` | `{ link, level:"info"\|"warn"\|"critical", message }` |
| `link-state` | `{ link, connected }` |
| `params-progress` | `{ sys, comp, received, expected }` |
| `param-value` | `ParamItem { index, name, value }` |

## 本机构建 (Windows + MSVC + WebView2)

> 当前 headless 服务器只能 `cargo check`，最终链接需在本机带 WebView2 的 Windows 桌面完成。

1. 安装依赖：
   - [Rust (MSVC toolchain)](https://rustup.rs/)
   - [Node.js 18+](https://nodejs.org/)（提供 npm / Vite）
   - [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)（Windows 11 通常已内置）
   - Visual Studio Build Tools (C++ 桌面开发 workload)
2. 安装前端依赖：
   ```bat
   npm --prefix ui-web install
   ```
3. 开发模式（热重载，前端跑在 `http://localhost:5173`）：
   ```bat
   cargo tauri dev
   ```
4. 生产打包（生成 `target/release/bundle/nsis/GroundControl_0.1.0_x64-setup.exe`）：
   ```bat
   cargo tauri build
   ```

## 安装与直接运行

- 打包产出的是 **NSIS 安装器**（`*_x64-setup.exe`），单文件分发。
- 双击安装器 → 安装到用户目录（`%LOCALAPPDATA%\GroundControl\`，**无需管理员权限**）→
  桌面与开始菜单自动生成 `GroundControl` 快捷方式，点击即可运行。
- 安装目录内的 `GroundControl.exe` 本身即可**直接双击运行**（所有依赖都在同目录），
  可整体复制该文件夹当作**便携版**使用。
- 配置已把 WebView2 引导程序**嵌入**安装器（`webviewInstallMode: embedBootstrapper`），
  目标机器若未装 WebView2 也会在安装时自动补齐，无需用户手动下载。
- 若需要真正的「单个 exe 免安装」文件：安装后用 Enigma Virtual Box 把安装目录
  封装为单 exe，或用 NSIS 单文件模式重打包——超出 Tauri 自身能力，需在本机额外处理。

## 连接示例

- **仿真 (SimLink)**：`kind=sim`，`bind=0.0.0.0:14551`（默认），适用于 SITL / 回环测试。
- **UDP**：`kind=udp`，`bind=0.0.0.0:14551`，`target=127.0.0.1:14550`。
- **串口**：`kind=serial`，`bind=COM8`，`target=57600`（波特率）。

## 说明

- `TelemetryHub` 为共享句柄（`Clone`），Tauri `AppState` 持有其一，订阅任务克隆后用于后台推送。
- 前端的 `ConnectKindJson` 用小写变体名（`sim`/`udp`/`serial`）以匹配 `<select>` 的 `option value`。
- `run.log` / `ui-web/node_modules` / `ui-web/dist` 已在 `.gitignore` 忽略。
