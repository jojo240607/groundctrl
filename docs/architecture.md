# 飞控地面站（Ground Control）软件架构设计

> 技术栈：**Rust + egui/Slint** | 协议：**MAVLink 2.0（兼容 1.0）**
> 目标平台：**Windows / Linux / macOS / Android / iOS**

---

## 一、总体定位与目标

| 维度 | 要求 |
|------|------|
| 协议 | MAVLink 2.0（兼容 1.0），支持 common / ardupilotmega 等 dialect |
| 跨平台 | Windows / Linux / macOS / Android / iOS（桌面 + 移动端平板/手机） |
| 核心能力 | 链路管理、遥测解析、地图/航迹、参数配置、航点规划、飞行监控、日志回放、指令下发 |
| 形态 | PC 端桌面应用 + 移动端 App，共用同一套 Rust 核心 |

---

## 二、为什么选 Rust + egui/Slint

### 2.1 Rust 适配性

| 维度 | Rust 表现 |
|------|-----------|
| 跨平台（Win/Linux/macOS/Android/iOS） | 一等公民，官方 `std` 全平台支持；Android/iOS 有官方编译目标 |
| MAVLink 协议 | 成熟生态：`mavlink` crate（官方 dialect 代码生成）、与 `pymavlink` 同源 |
| 内存/线程安全 | 编译期借用检查，飞控链路无数据竞争、无野指针 |
| 二进制体积/性能 | 接近 C，无 GC，适合实时遥测调试工具 |
| 移动端集成 | 核心编译为 `cdylib`/`staticlib`，经 FFI 接入原生 UI（无 Qt 商业授权成本） |
| 许可成本 | MIT/Apache，无商业限制，移动端免费 |

### 2.2 UI 方案：egui / Slint

- **egui**：纯 Rust Immediate-Mode GUI，跨平台一致性最好，最适合 HUD/仪表盘/调试面板，迭代快。
- **Slint**：Rust 声明式 UI，响应式布局强、商业友好，适合复杂表单/设置页。
- **策略**：以 **egui** 作为统一跨平台 UI 主框架（桌面 + 移动端均可运行），复杂业务页可局部采用 Slint；移动端也可选择原生 UI + Rust 核心 FFI。本期先以 egui 全 Rust 方案推进，保证五端一致。

### 2.3 关键技术选型

| 关注点 | 选型 |
|--------|------|
| 异步运行时 | `tokio`（统一事件循环，链路并发收发） |
| MAVLink | `mavlink` crate（`read_v2_msg` / `write_v2_msg` + dialect） |
| 串口 | `tokio-serial`（桌面）；Android 经 `jni` 桥接 `UsbSerial`；iOS 走网络数传 |
| 网络 | `tokio` UDP / TCP / WebSocket（SiK、WiFi、4G/5G 数传） |
| 序列化/配置 | `serde` + `toml` / `json` |
| 日志 | `tracing` + `tracing-subscriber` |
| UI | `egui` + `eframe`（桌面）；移动端 `egui` 经原生窗口宿主 或 原生 + FFI |
| FFI（移动端） | `uniffi` 或 `cbindgen` 生成绑定 |

---

## 三、架构总览（分层 + 跨平台抽象）

```
┌─────────────────────────────────────────────────────────────┐
│                       UI 层 (egui / Slint)                    │
│  Desktop: eframe  │  Android: egui-on-原生窗口  │  iOS: 同上   │
├─────────────────────────────────────────────────────────────┤
│                  应用逻辑层 (纯 Rust, 跨平台)                  │
│  VehicleModel │ MissionPlanner │ ParamManager │ TelemetryHub │
│  FlightMonitor │ LogPlayer │ CommandConsole                   │
├─────────────────────────────────────────────────────────────┤
│                   MAVLink 服务层 (纯 Rust)                    │
│  mavlink_router │ parser/encoder │ heartbeat │ message_dispatch│
├─────────────────────────────────────────────────────────────┤
│              平台抽象层 (trait 契约)                          │
│  Link(串口/网络/模拟) │ Storage │ Geo │ AsyncRuntime         │
├─────────────────────────────────────────────────────────────┤
│              传输层 (按平台实现)                              │
│  Serial: tokio-serial / Android USB-OTG / iOS Network        │
│  Network: UDP/TCP/WebSocket（WiFi / 4G / 5G 数传）           │
└─────────────────────────────────────────────────────────────┘
```

---

## 四、模块划分与职责

### 4.1 传输/链路层（Link Layer）

```rust
trait Link: Send {
    async fn send(&self, bytes: &[u8]) -> Result<()>;
    async fn recv(&self) -> Result<Vec<u8>>;
    fn quality(&self) -> LinkQuality; // RSSI / 丢包率 / 带宽
}
```

- `SerialLink`：串口 / USB-OTG（`tokio-serial`）
- `UdpLink`：局域网 / 地面 WiFi
- `TcpLink`：长距离数传 / 4G 隧道
- `SimLink`：软件在环 SITL 调试

职责：收发字节流、自动重连、带宽统计、链路质量评估。

### 4.2 MAVLink 服务层

- `MavlinkRouter`：多链路聚合，按 `system_id`/`component_id` 路由到对应 `Vehicle`。
- `MavlinkParser`：字节流 → 消息对象（封装 `mavlink` crate 的 `read_v2_msg`）。
- `HeartbeatManager`：发心跳、监测飞控在线状态（超时判定失联）。
- `MessageDispatcher`：基于 `tokio::sync::broadcast` 的发布/订阅，UI 与逻辑解耦。

### 4.3 应用逻辑层（与平台无关）

| 模块 | 职责 |
|------|------|
| `VehicleModel` | 单架飞机状态（姿态、位置、电池、模式、GPS） |
| `ParamManager` | 参数读取/缓存/写回，支持参数文件导入导出 |
| `MissionPlanner` | 航点增删改、上传/下载航点、FENCE、RALLY |
| `FlightMonitor` | 实时告警（低电量、失控、地理围栏越界） |
| `LogManager` | 飞行日志（tlog/bin）、回放、导出 KML |
| `CommandConsole` | 手动指令发送（MAV_CMD、飞控 shell） |

### 4.4 UI 层（egui，按平台宿主）

- **地图**：桌面用 `egui` + 地图瓦片（离线缓存）；移动端同源。
- **HUD**：姿态仪、空速/高度、电池用 egui 绘制。
- **布局**：响应式，移动端隐藏高级面板。

---

## 五、跨平台工程结构

```
groundctrl/
├── core/                  # 纯 Rust 库 (crate: groundctrl-core)
│   ├── mavlink/           # dialect 封装 + router + parser
│   ├── link/              # Link trait + serial/net/sim 实现
│   ├── vehicle/           # Vehicle, Params, Mission
│   ├── services/          # heartbeat, telemetry hub, log, alarms
│   └── proto/             # 内部消息总线 (tokio mpsc / broadcast)
├── ui-desktop/            # egui 桌面 App (Win/Linux/macOS, eframe)
├── ui-android/           # Kotlin + JNI 调 core (cdylib)
├── ui-ios/               # Swift + FFI 调 core (staticlib)
├── bindings/             # 自动生成 FFI (cbindgen / uniffi)
├── tools/                # 日志分析、SITL 模拟器 (纯 Rust)
├── Cargo.toml            # workspace
└── docs/                 # 设计文档
```

构建系统：Cargo workspace 主导；移动端通过 `uniffi`/`cbindgen` 生成 FFI 头，原生 UI 调用。

---

## 六、关键跨平台难点与对策

| 难点 | 对策 |
|------|------|
| iOS 不允许直接 USB 串口（MFi 限制） | iOS 端走网络数传（WiFi/4G 透传盒）或 MFi 认证模块 |
| Android USB-OTG 权限 | `UsbManager` + 后台服务保活链路，经 JNI 送入 Rust core |
| 地图离线 | 桌面/移动均支持离线瓦片缓存 |
| UI 自适应 | egui 响应式布局，移动端隐藏高级面板 |
| 后台保活（移动端收数传） | 前台服务 + 链路常驻，息屏不断网 |
| 核心跨端复用 | core 编译为 `cdylib`(Android) / `staticlib`(iOS)，原生 UI 经 FFI 调用 |

---

## 七、分阶段落地路线

1. **Phase 1**：`core` + `tokio` 链路 + `mavlink` 解析 + egui 桌面最小 Demo（连飞控看遥测）。
2. **Phase 2**：参数/航点/地图 HUD/日志（仍桌面）。
3. **Phase 3**：抽 FFI，做 Android 原生壳。
4. **Phase 4**：iOS 壳 + 网络数传适配。
5. **Phase 5**：多机、地理围栏、脚本任务。

---

## 八、设计原则

- **核心零平台依赖**：`core` crate 不依赖任何平台 API，仅依赖 `tokio`/`mavlink`/`serde` 等纯 Rust 库。
- **接口契约化**：链路、存储、地图经 `trait` 抽象，平台相关实现可插拔。
- **消息总线解耦**：UI 只订阅 `broadcast` 通道，不直接操作链路。
- **失败可恢复**：链路断开自动重连，不影响 UI 线程。
