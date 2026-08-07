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
- [x] `core/vehicle/params.rs`：`ParamManager`（读取/缓存/写回）— PARAM_REQUEST_LIST/READ/SET 协议封装
- [x] 参数列表 UI（表格、重新请求、写回）— `ui-desktop` Params 面板
- [x] 参数变更乐观更新 + 飞控回执校正

### 2.2 航点规划
- [x] `core/vehicle/mission.rs`：`MissionPlanner`（航点增删改、上传/下载握手、FENCE/RALLY 预留）
- [x] 航点编辑器 UI（列表 + 经纬度/高度输入 + 上传）— `ui-desktop` Mission 面板
- [x] 上传进度（MISSION_COUNT + 逐条 ITEM_INT）

### 2.3 地图与 HUD
- [x] 简化地图 HUD（离线，无瓦片）：以当前位置为中心绘制航迹线 + 当前点 — `ui-desktop` Map 面板
- [x] 飞机位置/航迹叠加显示（GPS 轨迹历史缓存于 UiState.trail）
- [x] 姿态仪升级为完整人工地平仪（artificial horizon）：天空/地面填充、roll 旋转、pitch 偏移、俯仰刻度、固定机体符号 — `ui-desktop` Telemetry 面板
- [x] 地图增强：缩放滑块（以当前点为中心动态视野）、本地航点叠加（橙色方块）、指北针（左上 N 指示器）— `ui-desktop` Map 面板
- [ ] 完整 HUD：空速表、高度表、油门/卫星数（基础遥测已显示，空速/高度仪表待做）
- [ ] 集成地图瓦片组件（egui 瓦片地图 + 离线缓存，需联网/缓存，留待后续）

### 2.4 日志
- [x] `core/services/log.rs`：`LogManager`（tlog 格式记录，每帧时间戳 + 原始 v2 字节）
- [x] 日志回放器（from_tlog 解码 → 驱动回调；已端到端测试 round-trip）
- [x] 导出 tlog 文件 / 从文件载入（`LogManager::save_file` / `load_file`）
- [x] 日志面板 UI：tlog 保存/加载（rfd 文件对话框）+ 轨迹 CSV 导出 + 轨迹/航点 KML 导出 — `ui-desktop` Log 面板
- [x] attach 时默认开启 recording（实时记录所有链路帧，可被 set_logging 关闭）

### 2.5 告警监控
- [x] `core/services/alarms.rs`：`FlightMonitor`（低电量 / 失联 / 围栏越界，去重）
- [x] 告警 UI 状态条（顶部，按等级红/黄着色）— `ui-desktop` alarm_bar

### 2.6 验证
- [x] SimLink 全流程：连模拟飞控，读参数（5 示例参数集满）、传航点（MISSION_COUNT+ITEM）、飞行监控（电量/围栏单元覆盖）、录日志（tlog round-trip）、回放 — 见 `core/tests/e2e_simlink.rs`
- [x] 全 workspace 编译 + 7 测试通过
- [ ] 实连飞控（串口/UDP）参数/航点/日志回放的真机验证 — 待真机/SITL 环境

**Phase 2 交付**：功能完整的桌面地面站。

---

## Phase 3 — Android 端

**目标**：Android App 复用 core，支持 USB-OTG / 网络数传。

> **暂缓**：用户已明确 Android/iOS 端先不做，等 Windows 桌面端（ui-desktop）完全跑通验证稳定后再推进。本阶段条目保留供后续实施。

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

> **暂缓**：与 Android 同理，iOS 端待 Windows 桌面端验证完成后再做。

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

### 坑 4：mavlink `common` 实际字段类型与文档直觉不同
- **PARAM_VALUE_DATA**：`param_id` 是 `[u8; 16]`（**不是** `[i8; 16]`），
  `param_count`/`param_index` 是 `u16`（**不是** `i16`）。早期想当然写成 i8/i16 数组导致 E0308 类型不匹配。
- **MISSION 系列结构没有 `mission_type` 字段**（至少 0.11.x 的 `MISSION_ITEM_INT_DATA` /
  `MISSION_REQUEST_LIST_DATA` / `MISSION_CLEAR_ALL_DATA` / `MISSION_COUNT_DATA` 均无此字段）。
  误加该字段会报“struct has no field `mission_type`”。需要删除该字段（或改用新版 dialect）。
- **修复**：直接用 `crate::vehicle::params::string_to_cstr`（返回 `[u8;16]`）构造 param_id；
  Mission 构造器仅填协议实际存在的字段。

### 坑 5：SimLink 用绝对 Unix 时间戳做电量衰减 → i8 溢出 + 告警误触发
- **现象**：Phase 2 联调时 `FlightMonitor` 在早期帧就报 `BATT_CRIT`（Critical 低电量），
  尽管 SimLink 初始电量设 80%。
- **根因 1**：`SimLink` 用 `SystemTime::now().as_secs_f64()`（绝对 Unix 秒，~1.7e9）直接
  做 `80 - (t as i32 / 2)`，`as i32` 虽不溢出但该值很大、取模后落在极低区间 → 电量被算成 ≤15 → 误报。
- **根因 2（更隐蔽）**：`VehicleModel::Battery.remaining_pct` 原本是 `i8`，`Default` 派生为 **0**。
  `FlightMonitor` 在收到第一帧真实 `SYS_STATUS` 之前先对默认 `Battery{remaining_pct:0}` 调用了
  `evaluate`，`0 <= critical(15)` 直接误触发并把 `"BATT_CRIT"` 插入活跃集合，污染了后续事件。
- **修复**：
  1. SimLink 记录 `start` 时刻，电量衰减改用 `(elapsed_sec as i32 / 2) % 80` 相对运行秒数，限制在 [5,80]。
  2. `Battery.remaining_pct` 改为 `Option<i8>`（默认 `None`=未知），`FlightMonitor` 用
     `if let Some(rem) = ...` 判断，未知时跳过且不报低电量。从根上消除“默认值 0 被当成真实 0%”。

### 坑 6：BusEvent 中途断言 complete 导致测试误 FAIL
- **现象**：`param_pull_over_simlink` 测试每收到一个 `Params` 事件就 `assert!(complete)`，
  而 SimLink 回放 5 个 PARAM_VALUE，前 4 个 `received<5` 时 `complete=false` → 测试 FAIL。
- **修复**：测试只取**最后一次** `Params` 事件（received 最大）的 `complete` 再断言，
  或汇总所有事件后判断最终是否集满。参数拉取本身是正确逐步收敛的。

### 坑 7：UI 导出/姿态仪实现要点（桌面端增强）
- **姿态仪（artificial horizon）**：`egui::Painter` 在 `allocate_painter` 返回需 `mut`；裁剪用
  `painter.set_clip_rect(rect)`（非旧版 `rect_clip`）。姿态旋转用闭包 `rot(x,y) -> Pos2`
  （返回 `Pos2` 不是 `Vec2`，否则 `line_segment`/`text` 类型不匹配）。天空/地面用
  `Shape::convex_polygon` 填充，`pitch` 单位是 f32 弧度，`clamp` 须用 f32 常量。
- **地图 HUD**：以当前点为中心、zoom 决定视野半宽（`half_span = 1/zoom² + 0.0008`），
  北在上的 Y 翻转映射；航点用 `rect_filled`，指北针画在 `resp.rect.min + offset`。
- **日志导出**：`LogManager` 需 `#[derive(Clone)]`（加载 tlog 后用 `lm.clone()` 替换 hub.log）。
  `save_file(&str)` 接收 `&str`，传 `path.to_str().unwrap()`；`load_file(&str) -> io::Result<LogManager>`
  返回新实例。UI 用 `rfd::AsyncFileDialog`（0.14）异步取路径，回调里经 `app.rt.handle().clone().spawn`
  跑；`hub.log().lock().await` 会因临时 `Arc` 借用报错，需先 `let log_arc = hub.log();` 再 `log_arc.lock().await`。
- **默认记录**：`TelemetryHub::attach` 里 `log.lock().await.set_recording(true)` 默认开启实时记录，
  否则 `log_frames` 恒为 0、导出为空（record 内部 `if !recording { return; }` 拦截）。

---

## 建议执行顺序（下一步）

从 **Phase 1 / 1.1 + 1.2 + 1.3 + 1.4 + 1.5 + 1.6** 开始，先产出可编译运行的最小桌面 Demo，验证 MAVLink 全链路后再逐步扩展。当前 1.1–1.5 核心代码已就位，下一步重点是 **1.6 验证**：用 `core` 集成测试跑通 SimLink → TelemetryHub → VehicleModel 全链路，并确认 `ui-desktop` 在图形环境下显示遥测。
