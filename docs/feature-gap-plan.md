# 地面站功能补齐计划（Feature Gap Plan）

> 日期：2026-08-10
> 目标：对照商用地面站（QGroundControl / Mission Planner），识别功能差距并按优先级补齐。

---

## 一、现状盘点（已实现）

### 链路层
- 串口 `SerialLink` / UDP `UdpLink` / 模拟 `SimLink`，链路质量统计（收包/丢包/带宽）、自动重连

### 遥测解析（下行）
- HEARTBEAT（模式/armed/在线判定）、ATTITUDE、GLOBAL_POSITION_INT、SYS_STATUS（电压/电流/余电）、VFR_HUD（空速/地速/爬升/油门）、PARAM_VALUE
- 多机聚合（按 system_id 的 fleet）

### UI 面板（egui 桌面端）
| 面板 | 功能 |
|------|------|
| Telemetry | 人工地平仪、空速/高度圆表、GPS/电池卡片、模式/armed 徽章 |
| Map | 离线瓦片（目录加载）、航迹、航点叠加/拖拽、朝向三角形、指北针 |
| Mission | 航点增删改/排序、上传/下载（MISSION 握手） |
| Params | 参数拉取/写回/乐观更新、加入趋势 |
| Trends | 参数实时折线图（1Hz，300 点） |
| Log | tlog 录制/回放、CSV/KML 导出 |
| Alarms | 低电量/失联/围栏越界（阈值可配置）+ 历史列表 |
| Settings | 链路/瓦片目录/告警规则配置持久化 |
| Connection | 串口/UDP/Sim 连接管理 |

另有 `groundctrl-tauri` 壳（IPC 命令覆盖连接/机队/参数/航点/监控，前端未开发）。

---

## 二、与商用地面站差距清单

### P0 — 核心缺失（无则不可正常飞行操控）
| # | 功能 | 说明 |
|---|------|------|
| P0-1 | 飞行控制指令 | 无 ARM/DISARM、TAKEOFF、LAND、RTL、模式切换。仅底层 `send_msg`，无 COMMAND_LONG 封装与 UI | ✅ |
| P0-2 | 在线地图 | 仅离线瓦片目录，无内置网络瓦片源（OSM/高德等）在线加载 | ✅（已存在） |
| P0-3 | 遥控通道显示 | 无 RC_CHANNELS 解析，看不到遥控器状态 | ✅ |

### P1 — 重要缺失
| # | 功能 | 说明 | 状态 |
|---|------|------|------|
| P1-1 | 数据流速率控制 | 无 REQUEST_DATA_STREAM / SET_MESSAGE_INTERVAL | ✅ |
| P1-2 | GPS 详情 | 有 fix/sats 字段但无卫星数/HDOP/精度显示 | ✅ |
| P1-3 | 任务文件管理 | 无航点保存/加载（.plan / KML 导入） | ✅ |
| P1-4 | 围栏上传 | FENCE/RALLY 仅本地告警，未实现上传协议 | ✅ |

### P2 — 增强
| # | 功能 | 说明 | 状态 |
|---|------|------|------|
| P2-1 | 日志图表分析 | tlog 回放后曲线分析工具 | ✅ |
| P2-2 | 校准向导 | 磁罗盘/加速度计/水平校准流程 | ✅ |
| P2-3 | 摇杆/键盘操控 | 摇杆映射、键盘模式切换 | ✅ |
| P2-4 | 声音告警 | 告警提示音 | ✅ |

### P3 — 长尾
| # | 功能 | 说明 | 状态 |
|---|------|------|------|
| P3-1 | 固件升级 | MAVLink FTP 客户端（core）+ SimLink 服务端 + e2e + UI 面板 | ✅ |
| P3-2 | 视频 / OSD | UDP MJPEG 接收 + 解码显示 + OSD 叠加 + 模拟图传源 | ✅ |
| P3-3 | i18n | zh-CN / en-US 语言字典 + 设置切换 + 主要 UI 文本替换 | ✅ |
| P3-4 | 脚本任务 | 迷你任务 DSL（WAIT/ARM/TAKEOFF/IF 等）+ 执行器 + e2e + UI 面板 | ✅ |

---

## 三、实施路线（按优先级依次补齐）

### 阶段 1（P0-1）：飞行控制指令
- **core**：`TelemetryHub` 增加 COMMAND_LONG 发送 API（`send_command_long` / `send_arm_disarm` / `send_takeoff` / `send_land` / `send_rtl` / `send_mode`），构造 `COMMAND_LONG_DATA` 经 `write_v2_msg` 下发
- **core**：`VehicleModel::apply` 处理 `COMMAND_ACK` 回执，记录最近指令结果（成功/拒绝/超时）
- **ui-desktop**：Telemetry 面板新增控制区——ARM/DISARM、TAKEOFF、LAND、RTL 按钮 + 模式下拉（STABILIZE/ALT_HOLD/LOITER/RTL/LAND/GUIDED）+ ACK 状态显示
- **验收**：SimLink 回环收到 COMMAND_LONG 并回 ACK，UI 显示指令结果

### 阶段 2（P0-2）：在线瓦片地图
- **core/settings**：新增瓦片 URL 模板配置（如 OSM `https://tile.openstreetmap.org/{z}/{x}/{y}.png`、高德等）
- **ui-desktop Map**：在线模式按瓦片坐标请求 URL → 解码 → 纹理缓存（复用现有 `tile_cache`）；离线目录优先、在线为默认
- **验收**：联网时 Map 面板显示底图，断网回退 HUD 模式

### 阶段 3（P0-3）：RC 通道显示
- **core**：`VehicleModel` 增加 `rc: RcChannels { ch: [u16; 8], rssi: u8, mode: u8 }`，`apply` 处理 `RC_CHANNELS`
- **ui-desktop**：Telemetry 面板新增遥控状态卡片（通道 1-8 条形图、信号强度、失联提示）
- **验收**：SimLink 合成 RC_CHANNELS，UI 显示通道值变化

### 阶段 4（P1-1 + P1-2）：数据流控制 + GPS 详情
- **core**：`request_data_stream(sys, comp, stream, rate)` / `set_message_interval` API
- **ui-desktop**：Telemetry GPS 卡片扩展（fix_type 名称、卫星数、HDOP/精度、速度）
- **验收**：可调整下行速率，GPS 卡片信息完整

### 阶段 5（P1-3）：任务文件管理
- **ui-desktop Mission**：航点保存/加载 `.plan`（JSON）与 KML 导入导出
- **core**：`mission::Waypoint` 序列化支持
- **验收**：保存 → 加载 → 上传全流程

### 阶段 6（P1-4）：围栏上传
- **core**：FENCE 消息族（FENCE_POINT / FENCE_FETCH_POINT / FENCE_STATUS）封装
- **ui-desktop**：Map 围栏编辑 + 上传/下载
- **验收**：SimLink 回环验证围栏点上传下载一致

### 阶段 7（P2）：增强项 ✅ 全部完成
- 图表分析（tlog → 曲线）、校准向导（CALIBRATE 命令族）、摇杆/键盘映射、声音告警（rodio）

### 阶段 8（P3）：长尾 ✅ 全部完成
- P3-1 固件升级：MAVLink FTP 客户端/服务端回环、UI 固件面板（上传/下载/CRC32/删除/列表）
- P3-2 视频/OSD：UDP MJPEG 接收（FFD8/FFD9 组帧）+ egui 解码显示 + OSD 叠加 + 模拟视频源
- P3-3 i18n：中文即 key 的轻量字典，设置面板语言切换（zh-CN/en-US）并持久化
- P3-4 脚本任务：脚本引擎（core）+ 执行面板（ui-desktop），支持条件分支与中止

---

## 四、验收标准（总）
1. 每个阶段 `cargo build` / `cargo test` 全绿，无新增编译错误
2. SimLink 回环端到端测试覆盖新功能（core/tests/e2e_simlink.rs 扩展）
3. 与 joc-app-rust 飞控实机/模拟联调验证指令链路
