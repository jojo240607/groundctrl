# GroundControl — Tauri (Web) 桌面端

`groundctrl-tauri` 是基于 **Tauri v2** 的桌面地面站，后端为 Rust（复用 `groundctrl-core`），
前端为原生 HTML/CSS/JS（`ui-web/`，无运行时框架，经 Vite 打包）。

## 地图底图

地图使用 **Leaflet**（从 npm 本地打包，运行时零 CDN 依赖）叠加 **OpenStreetMap 标准瓦片**
（`https://tile.openstreetmap.org/{z}/{x}/{y}.png`，开源、无需 API key），与 Mission Planner /
QGroundControl 同款做法——航点、飞机、围栏直接用真实 WGS84 经纬度叠加，缩放/拖拽/拖拽航点都由
Leaflet 原生处理。

- **联网时**：直接显示真实街道/地形底图。
- **离线或被网络拦截时**：瓦片加载失败，地图左上角显示「离线：地图瓦片不可用」提示，
  但航点编辑、坐标读数、飞机定位、围栏绘制等所有功能仍正常工作（只是没有底图）。
- 瓦片恢复可达后自动隐藏提示并加载底图。

如需卫星影像底图，把 `ui-web/map.js` 中 `L.tileLayer` 的 URL 换成 ESRI World Imagery
（`https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}`）
或 OpenAerialMap 等开源源即可（均为无 key 的开源瓦片）。

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
| `list_serial_ports` | — | `[String]`（可用串口名，如 `["COM3","COM9"]`） |
| `disconnect` | — | `void` |
| `get_fleet` | — | `FleetSnapshot` |
| `get_settings` / `save_settings` | `SettingsJson` | `SettingsJson` / `void` |
| `get_monitor_config` / `set_monitor_config` | `MonitorConfigJson` | `MonitorConfigJson` / `void` |
| `request_params` | `{sys, comp}` | `void` |
| `set_param` | `{sys, comp, name, value}` | `void` |
| `upload_mission` | `{sys, comp, items:[WaypointItem]}` | `void` |
| `download_mission` | `{sys, comp}` | `[WaypointItem]`（向飞控发 MISSION_REQUEST_LIST 并逐条接收 MISSION_ITEM_INT，返回完整航点；超时返回已收到的部分） |

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

> 实测：本仓库当前 Rust 工具链为 **GNU (`x86_64-pc-windows-gnu`) + MinGW**，
> Tauri v2 在该环境下 `cargo tauri build` 也能成功产出 exe（链接器会有
> `corrupt .drectve` 的无害告警）。若使用 **MSVC 工具链** 同样可行且告警更少。

1. 安装依赖：
   - Rust（GNU 或 MSVC 均可；GNU 需 MinGW，MSVC 需 VS2022 生成工具）
   - [Node.js 18+](https://nodejs.org/)（提供 npm / Vite）
   - [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)（Windows 11 通常已内置；缺则安装器会自动补齐）
2. 一键打包（推荐）：直接双击仓库根目录 `build.bat`，或运行 `build.ps1`。
   手动步骤：
   ```bat
   cargo install tauri-cli --version "^2"   :: 仅首次
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

## USB CDC 直连飞控

自研飞控（flyctrl-core + joc-app-rust @ 自研 RTOS）通过 **USB CDC-ACM 虚拟串口** 与地面站通信。
飞控 `telemetry` 任务优先把 **MAVLink v2**（`0xFD`，标准 `common` dialect、含 `CRC_EXTRA`）
写入 `usb0` 设备（USB 未枚举时回退到 `uart3`），因此插上 USB 即被识别为一个 `COMx`，
地面站直接以串口方式打开即可，无需额外转换线。

### 对接前提（已对齐）
- **协议版本一致**：飞控已升级为 MAVLink v2（`0xFD`），地面站 `core/src/mlink` 用
  `read_v2_msg` 解析，双方 `common` dialect 的 `CRC_EXTRA` 字节级一致（HEARTBEAT=50、
  SYS_STATUS=124、LOCAL_POSITION_NED=143、ATTITUDE=39、COMMAND_LONG=152 等），双向可通。
- **通道一致**：飞控下行走 `usb0`（USB CDC），地面站串口链路用 `tokio-serial` 打开该 `COMx`。
- **波特率忽略**：USB CDC 是虚拟串口，物理层无波特率，前端填任意值（如 `115200`）均可。

### 操作步骤
1. 板子上电，用 USB 数据线连接电脑。Windows 设备管理器「端口 (COM 和 LPT)」下会出现一个新
   `COMx`（usbser.sys 原生驱动，无需额外 INF；每次插拔 `COM` 号可能变化）。
2. 启动地面站（`cargo tauri dev` 或安装版 `GroundControl.exe`）。
3. 「连接」卡片：连接类型选 **串口** → 点 **刷新** 按钮，端口下拉自动枚举出可用 `COMx`
   （底层调用 `list_serial_ports` 命令，`serialport::available_ports()`）→ 选到板子对应的 `COMx`
   （选中后自动填入后端需要的 `bind` 字段）→ 波特率填 `115200` → 点 **连接**。
4. 连接成功后：`link-state` 事件 `connected=true`，`fleet` 快照开始推送，地图出现飞机图标、
   姿态仪表与趋势曲线刷新、参数面板可 `request_params` 拉取飞控参数。

### 排查
- 下拉里看不到 `COMx`：确认 USB 已连、设备管理器有该端口、且飞控 `usb0` 已枚举
  （`telemetry` 任务会先 `USB_IOCTL_CONNECTED` 探测再写）。
- 连上但 `fleet` 无数据：先用串口助手（如 PuTTY/Arduino 串口监视器，任意波特率）打开该 `COMx`，
  应看到二进制 MAVLink v2 帧（首字节 `0xFD`）；若看不到，是飞控侧未吐数据（检查 `usb0` 枚举与
  `telemetry` 任务是否在跑），与地面站无关。
- 收得到帧但解析异常：确认飞控/地面站均为 MAVLink v2；若飞控仍为 v1（`0xFE`），
  需把地面站 `mlink` 解析改为 v1/v2 自动识别（见 `core/src/mlink/mod.rs`）。


## 说明

- `TelemetryHub` 为共享句柄（`Clone`），Tauri `AppState` 持有其一，订阅任务克隆后用于后台推送。
- 前端的 `ConnectKindJson` 用小写变体名（`sim`/`udp`/`serial`）以匹配 `<select>` 的 `option value`。
- `run.log` / `ui-web/node_modules` / `ui-web/dist` 已在 `.gitignore` 忽略。
