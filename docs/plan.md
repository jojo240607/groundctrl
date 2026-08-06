# 飞控地面站 — 详细执行计划

> 技术栈：Rust + egui/Slint + tokio + mavlink crate
> 目标平台：Windows / Linux / macOS / Android / iOS
> 关联文档：`docs/architecture.md`

---

## Phase 1 — 核心链路 + 桌面遥测 Demo（MVP）

**目标**：连上飞控，桌面端用 egui 显示心跳、姿态、电池、GPS 等基础遥测。

### 1.1 工程脚手架
- [x] 初始化 Cargo workspace：`Cargo.toml`（成员 `core`、`ui-desktop`、`tools`）
- [x] `core/Cargo.toml`：依赖 `tokio`、`mavlink`、`serde`、`tracing`、`thiserror`
- [x] `ui-desktop/Cargo.toml`：依赖 `eframe`、`egui`、`tokio`、`groundctrl-core`（path）
- [x] 目录结构按 `architecture.md` 第五节建立

### 1.2 MAVLink 服务层
- [x] 引入 `mavlink` crate，指定 dialect（`common` feature，workspace 统一）
- [x] `core/mlink/mod.rs`：封装 `read_v2_msg` / `write_v2_msg`（含 `MavlinkParser` 缓冲解析）
- [x] 路由：在 `TelemetryHub::attach` 内按 `system_id` 聚合到 `VehicleModel`（等价 router）
- [x] `core/mlink/mod.rs`：周期心跳构造 + 在线判定（由各链路 `is_open` 驱动）

### 1.3 链路层
- [x] `core/link/mod.rs`：定义 `trait Link`
- [x] `core/link/serial.rs`：`SerialLink`（`tokio-serial`）
- [x] `core/link/udp.rs`：`UdpLink`（`tokio` UDP）
- [x] `core/link/sim.rs`：`SimLink`（SITL / 回环测试数据源）
- [x] 链路质量统计（收包数、丢包率、带宽）

### 1.4 应用逻辑层（最小）
- [x] `core/vehicle/model.rs`：`VehicleModel`（姿态/位置/电池/模式/GPS）
- [x] `core/services/telemetry_hub.rs`：链路字节 → 解析 → 更新 `VehicleModel` → `broadcast` 发布
- [x] `core/proto/bus.rs`：`tokio::sync::broadcast` 消息总线定义

### 1.5 桌面 UI（egui）
- [x] `ui-desktop/main.rs`：eframe 启动 + tokio runtime 启动
- [x] 链路配置面板（串口/UDP/Sim 分组选择器 + 波特率 + 连接/断开按钮 + 当前链路状态显示；打开默认连 Sim）
- [x] 基础遥测面板（姿态文本、电池、GPS、模式、信号）
- [x] 简单姿态仪绘制（egui 2D）

### 1.6 验证
- [x] 用 `SimLink` 注入模拟消息，`core` 集成测试 `e2e_simlink` 端到端验证全链路通过
- [ ] 实连飞控（串口/UDP），确认心跳与遥测正常 —— 待真机/SITL 环境
- [x] Win/Linux/macOS 三端编译通过（`cargo check` 全 workspace 通过；实机运行待验证）

**Phase 1 交付**：可连接飞控、显示基础遥测的跨平台桌面程序。

---

## Phase 2 — 完整飞行功能（桌面）

**目标**：参数、航点、地图 HUD、日志回放。

### 2.1 参数管理
- [ ] `core/vehicle/params.rs`：`ParamManager`（读取/缓存/写回）
- [ ] 参数列表 UI（表格、筛选、读/写、导入导出文件）
- [ ] 参数变更确认与失败回执

### 2.2 航点规划
- [ ] `core/vehicle/mission.rs`：`MissionPlanner`（航点增删改、上传/下载、FENCE、RALLY）
- [ ] 航点编辑器 UI（列表 + 地图标点）
- [ ] 上传/下载进度与校验

### 2.3 地图与 HUD
- [ ] 集成地图组件（egui 瓦片地图 + 离线缓存）
- [ ] 飞机位置/航迹/航点叠加显示
- [ ] 完整 HUD：姿态仪、空速/高度、电池、油门、GPS 卫星数

### 2.4 日志
- [ ] `core/services/log.rs`：`LogManager`（tlog/bin 记录）
- [ ] 日志回放器（加载日志 → 驱动 `VehicleModel` 回放）
- [ ] 导出 KML / CSV

### 2.5 告警监控
- [ ] `core/services/alarms.rs`：`FlightMonitor`（低电量、失控、围栏越界）
- [ ] 告警 UI 弹窗/状态条

### 2.6 验证
- [ ] SITL 全流程：连模拟飞控，读参数、传航点、飞行监控、录日志、回放
- [ ] 三端编译与基础交互测试

**Phase 2 交付**：功能完整的桌面地面站。

---

## Phase 3 — Android 端

**目标**：Android App 复用 core，支持 USB-OTG / 网络数传。

### 3.1 FFI 抽取
- [ ] `bindings/`：用 `cbindgen` 或 `uniffi` 从 `core` 生成 Android 可调用的绑定
- [ ] `core` 提供 `cdylib` 产物 + 安全的 FFI 接口层（错误码、回调）

### 3.2 Android 原生壳
- [ ] `ui-android/`：Kotlin 项目（Gradle），加载 Rust 动态库
- [ ] USB-OTG：`UsbManager` 权限申请 + `jni` 桥接 `UsbSerial` → Rust `Link`
- [ ] 网络数传：UDP/TCP 直连 core
- [ ] 链路前台服务保活（息屏不断网）
- [ ] egui 宿主窗口 或 原生 UI 调用 FFI 渲染遥测

### 3.3 验证
- [ ] Android 实机：USB-OTG 连飞控数传，显示遥测
- [ ] 网络数传连通性测试

**Phase 3 交付**：可运行的 Android 地面站。

---

## Phase 4 — iOS 端

**目标**：iOS App 复用 core，走网络数传（iOS 无直接 USB 串口）。

### 4.1 FFI 抽取
- [ ] `core` 提供 `staticlib` 产物 + `cbindgen` 生成 C 头
- [ ] ObjC++ 桥接层连接 Swift 与 Rust core

### 4.2 iOS 原生壳
- [ ] `ui-ios/`：Swift 项目（Xcode），链接 Rust `staticlib`
- [ ] 网络数传：WiFi/4G 透传盒 UDP/TCP → core
- [ ] egui 宿主 或 SwiftUI 调用 FFI 渲染遥测
- [ ] 后台音频/定位保活链路（iOS 限制内）

### 4.3 验证
- [ ] iOS 实机：网络数传连飞控，显示遥测
- [ ] App Store 合规检查（无私有 API）

**Phase 4 交付**：可运行的 iOS 地面站。

---

## Phase 5 — 高级功能

- [ ] 多机管理（`MavlinkRouter` 支持多 `VehicleModel` 同屏）
- [ ] 地理围栏可视化编辑与实时越界保护
- [ ] 脚本/任务自动化（Lua 或 Rust 脚本引擎）
- [ ] 云台/相机控制、视频流叠加（可选 RTSP 解码）
- [ ] 固件升级（MAVLink FTP / 自定义）
- [ ] 国际化（i18n）、主题切换

---

## 风险与对策

| 风险 | 对策 |
|------|------|
| iOS USB 限制 | 仅网络数传，文档明确说明 |
| egui 移动端输入/分辨率适配 | 初期用原生 UI + FFI，egui 仅桌面；后续评估 egui 移动端成熟度 |
| Rust 移动端编译工具链 | 提前配置 `rustup target`：aarch64-linux-android / aarch64-apple-ios |
| MAVLink dialect 差异 | 以 `common` 为基，按需叠加厂商 dialect，路由层按 `system_id` 区分 |
| 链路并发安全 | `tokio` + `broadcast` 解耦，UI 不直接持有链路 |

---

## 调试记录（已踩坑）

### 坑 1：裸 `Result<T>` 误报 E0107 "enum takes 2 generic arguments"
- **现象**：`core/src/mlink/mod.rs` 写 `-> Result<Vec<u8>> {`，编译器报
  `error[E0107]: enum takes 2 generic arguments but 1 generic argument was supplied`，
  下划线指向 `Result<Vec<u8>>`，help 提示补 `, E>`。
- **根因**：该模块内**没有 `use crate::error::Result;`**，裸名 `Result` 回退到
  **std prelude 的 `std::result::Result<T, E>`**（确实是要 2 个参数的 enum），
  `Vec<u8>` 只提供了 `T`、缺了 `E`。错误信息里的 "enum" 正是 std 的 `Result`。
  （不要被同时出现的 `MavMessage` 名字误导——`MavMessage` 在本模块是合法的非泛型
  `type` 别名，本身没错；真正缺参数的是 `Result`。）
- **修复**：在用到 `Result` 的模块顶部显式 `use crate::error::Result;`
  （`crate::error::Result` 是 `type Result<T> = std::result::Result<T, GcError>`，1 参数）。
- **规则**：凡是写 `Result<...>` 的模块都要 `use crate::error::Result;`，
  不能依赖 `lib.rs` 里的 `pub use error::{GcError, Result}`——re-export
  **不会**把名字注入子模块的裸名解析。

### 坑 2：模块级 re-export 缺失导致 E0432
- **现象**：`ui-desktop` 写 `services::TelemetryHub`，编译报
  `error[E0432]: unresolved import` / `no TelemetryHub in services`。
- **根因**：`services` 模块只 `pub mod telemetry_hub;`（`telemetry_hub` 是模块名，
  内含 `pub struct TelemetryHub`）。外部用 `services::TelemetryHub` 会把结构体名
  当成模块名，解析失败。
- **修复**：在 `core/src/services/mod.rs` 加 `pub use telemetry_hub::TelemetryHub;`
  把结构体提升为服务层公共类型，外部即可 `services::TelemetryHub`。

### 坑 3：mavlink crate 仅启用 `common` feature
- workspace 统一 features = `std` + `common` + `format-generated-code`。
- `mavlink::common::MavMessage` 在该组合下是**非泛型扁平枚举**，
  用 `pub type MavMessage = ::mavlink::common::MavMessage;` 即可。
- 若改用 `ardupilotmega` 等 dialect 需额外在对应 crate 的 Cargo.toml 加 feature，
  并会让 `common::MavMessage` 变成泛型 `MavMessage<M, V>`（会触发本记录的坑 1 类错误）。
- 早期 `core/tests/probe.rs` 用 `mavlink::ardupilotmega::MavMessage` 已失效（无该 feature），
  已替换为仅依赖 `common` 的端到端验证测试。

---

## 建议执行顺序（下一步）

从 **Phase 1 / 1.1 + 1.2 + 1.3 + 1.4 + 1.5 + 1.6** 开始，先产出可编译运行的最小桌面 Demo，验证 MAVLink 全链路后再逐步扩展。当前 1.1–1.5 核心代码已就位，下一步重点是 **1.6 验证**：用 `core` 集成测试跑通 SimLink → TelemetryHub → VehicleModel 全链路，并确认 `ui-desktop` 在图形环境下显示遥测。
