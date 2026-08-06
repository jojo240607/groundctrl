# 飞控地面站 — 详细执行计划

> 技术栈：Rust + egui/Slint + tokio + mavlink crate
> 目标平台：Windows / Linux / macOS / Android / iOS
> 关联文档：`docs/architecture.md`

---

## Phase 1 — 核心链路 + 桌面遥测 Demo（MVP）

**目标**：连上飞控，桌面端用 egui 显示心跳、姿态、电池、GPS 等基础遥测。

### 1.1 工程脚手架
- [ ] 初始化 Cargo workspace：`Cargo.toml`（成员 `core`、`ui-desktop`、`tools`）
- [ ] `core/Cargo.toml`：依赖 `tokio`、`mavlink`、`serde`、`tracing`、`thiserror`
- [ ] `ui-desktop/Cargo.toml`：依赖 `eframe`、`egui`、`tokio`、`groundctrl-core`（path）
- [ ] 目录结构按 `architecture.md` 第五节建立

### 1.2 MAVLink 服务层
- [ ] 引入 `mavlink` crate，指定 dialect（`common` + `ardupilotmega` feature）
- [ ] `core/mavlink/parser.rs`：封装 `read_v2_msg` / `write_v2_msg`
- [ ] `core/mavlink/router.rs`：按 `system_id`/`component_id` 路由到 `Vehicle`
- [ ] `core/mavlink/heartbeat.rs`：周期发心跳 + 在线超时判定

### 1.3 链路层
- [ ] `core/link/mod.rs`：定义 `trait Link`
- [ ] `core/link/serial.rs`：`SerialLink`（`tokio-serial`）
- [ ] `core/link/udp.rs`：`UdpLink`（`tokio` UDP）
- [ ] `core/link/sim.rs`：`SimLink`（SITL / 回环测试数据源）
- [ ] 链路质量统计（收包数、丢包率、带宽）

### 1.4 应用逻辑层（最小）
- [ ] `core/vehicle/model.rs`：`VehicleModel`（姿态/位置/电池/模式/GPS）
- [ ] `core/services/telemetry_hub.rs`：链路字节 → 解析 → 更新 `VehicleModel` → `broadcast` 发布
- [ ] `core/proto/bus.rs`：`tokio::sync::broadcast` 消息总线定义

### 1.5 桌面 UI（egui）
- [ ] `ui-desktop/main.rs`：eframe 启动 + tokio runtime 启动
- [ ] 链路配置面板（选串口/网络、波特率、连接按钮）
- [ ] 基础遥测面板（姿态文本、电池、GPS、模式、信号）
- [ ] 简单姿态仪绘制（egui 2D）

### 1.6 验证
- [ ] 用 `SimLink` 注入模拟消息，UI 正确显示
- [ ] 实连飞控（串口/UDP），确认心跳与遥测正常
- [ ] Win/Linux/macOS 三端编译通过并运行

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

## 建议执行顺序（下一步）

从 **Phase 1 / 1.1 + 1.2 + 1.3 + 1.4 + 1.5** 开始，先产出可编译运行的最小桌面 Demo，验证 MAVLink 全链路后再逐步扩展。
