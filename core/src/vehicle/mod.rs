//! 飞行器状态模型（与平台无关）

pub mod params;
pub mod mission;

use serde::Serialize;

use crate::mlink;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Attitude {
    pub roll: f32,  // rad
    pub pitch: f32, // rad
    pub yaw: f32,   // rad
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GpsPos {
    pub lat: f64,        // deg
    pub lon: f64,        // deg
    pub alt: f32,        // m
    pub relative_alt: f32, // m
    pub heading: f32,    // deg
    pub fix_type: u8,
    pub satellites: u8,
    /// 水平定位精度 HDOP（米，来自 GPS_RAW_INT.eph）
    pub hdop: f32,
    /// 垂直定位精度（米，来自 GPS_RAW_INT.epv）
    pub vdop: f32,
    /// 地面速度（m/s，来自 GPS_RAW_INT.vel）
    pub ground_speed: f32,
    /// 是否收到过 GLOBAL_POSITION_INT / GPS_RAW_INT 帧
    pub seen: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Battery {
    pub voltage: f32,        // V
    pub current: f32,        // A
    /// 剩余电量百分比（None = 未知 / 飞控未上报）
    pub remaining_pct: Option<i8>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AirData {
    /// 真空速 (m/s)
    pub airspeed: f32,
    /// 地速 (m/s)
    pub groundspeed: f32,
    /// 垂直速度 (m/s, 爬升为正)
    pub climb: f32,
    /// 油门 (%) 0..100
    pub throttle: u16,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HeartbeatInfo {
    pub system_id: u8,
    pub component_id: u8,
    pub mav_type: u8,
    pub autopilot: u8,
    pub base_mode: u8,
    pub custom_mode: u32,
    pub system_status: u8,
    pub last_seen: u64, // ms 时间戳
}

/// 遥控器通道状态（来自 RC_CHANNELS）
#[derive(Debug, Clone, Default, Serialize)]
pub struct RcChannels {
    /// 前 8 个常用通道的原始 PWM 值（1-8 对应摇杆/开关）
    pub ch: [u16; 8],
    /// 飞控上报的通道总数（0 = 未知）
    pub chancount: u8,
    /// 遥控信号强度 0-254（255 = 无效），0 = 未知
    pub rssi: u8,
    /// 是否收到过 RC_CHANNELS 帧
    pub seen: bool,
    /// 最近帧时间戳（ms）
    pub last_seen: u64,
}

impl RcChannels {
    /// 归一化某通道值到 0..1（PWM 1000..2000 中心 1500）
    pub fn norm(&self, ch: usize) -> Option<f32> {
        let raw = self.ch.get(ch).copied()?;
        if raw == 0 {
            return None;
        }
        let v = (raw as f32 - 1000.0) / 1000.0;
        Some(v.clamp(0.0, 1.0))
    }

    /// 遥控器是否在线（收到帧且信号有效）
    pub fn link_ok(&self) -> bool {
        self.seen && self.rssi != 255 && self.rssi > 0
    }
}

/// 最近一次飞行指令的回执（来自 COMMAND_ACK）
#[derive(Debug, Clone, Default, Serialize)]
pub struct CommandAck {
    /// 指令编号（MAV_CMD）
    pub command: u16,
    /// 结果码（MAV_RESULT）
    pub result: u8,
    /// 回执时间戳（ms）
    pub ts: u64,
}
impl CommandAck {
    /// MAV_RESULT 可读名
    pub fn result_name(&self) -> String {
        match self.result {
            0 => "已接受".into(),
            1 => "暂忙".into(),
            2 => "被拒绝".into(),
            3 => "不支持".into(),
            4 => "失败".into(),
            5 => "进行中".into(),
            _ => format!("结果 {}", self.result),
        }
    }
}

/// 地理围栏状态（来自 FENCE_STATUS）
#[derive(Debug, Clone, Default, Serialize)]
pub struct FenceStatus {
    /// 是否收到过 FENCE_STATUS 帧
    pub seen: bool,
    /// 围栏违例状态（0 = 无违例，1 = 违例中）
    pub breach_status: u8,
    /// 累计违例次数
    pub breach_count: u16,
    /// 最近一次违例的时长（ms）
    pub breach_time: u32,
    /// 最近一次违例类型（FenceBreach：1=MIN_ALT 2=MAX_ALT 3=BOUNDARY）
    pub breach_type: u8,
    /// 最近帧时间戳（ms）
    pub last_seen: u64,
}

impl FenceStatus {
    /// 围栏是否生效（收到过状态帧即视为飞控已启用围栏功能）
    pub fn enabled(&self) -> bool {
        self.seen
    }

    /// 违例类型可读名
    pub fn breach_type_name(&self) -> &'static str {
        match self.breach_type {
            1 => "最低高度",
            2 => "最高高度",
            3 => "边界",
            _ => "未知",
        }
    }

    /// 状态栏文本
    pub fn status_text(&self) -> String {
        if !self.seen {
            return "未上报".into();
        }
        if self.breach_status != 0 {
            format!(
                "违例中（{}）",
                self.breach_type_name()
            )
        } else {
            format!("正常（已违例 {} 次）", self.breach_count)
        }
    }
}

/// 单架飞机完整状态快照
#[derive(Debug, Clone, Default, Serialize)]
pub struct VehicleModel {
    pub sys_id: u8,
    pub comp_id: u8,
    pub online: bool,
    pub heartbeat: Option<HeartbeatInfo>,
    pub attitude: Attitude,
    pub gps: GpsPos,
    pub battery: Battery,
    pub air: AirData,
    pub rc: RcChannels,
    /// 最近一条飞行指令的回执（None = 尚无回执）
    pub cmd_ack: Option<CommandAck>,
    /// 地理围栏状态（FENCE_STATUS）
    pub fence: FenceStatus,
    pub link_name: String,
}

impl VehicleModel {
    /// 应用一条已解析的 MAVLink 消息，更新自身状态
    pub fn apply(&mut self, header: &::mavlink::MavHeader, msg: &mlink::MavMessage) {
        self.sys_id = header.system_id;
        self.comp_id = header.component_id;
        self.online = true;

        match msg {
            mlink::MavMessage::HEARTBEAT(d) => {
                self.heartbeat = Some(HeartbeatInfo {
                    system_id: header.system_id,
                    component_id: header.component_id,
                    mav_type: d.mavtype as u8,
                    autopilot: d.autopilot as u8,
                    base_mode: d.base_mode.bits(),
                    custom_mode: d.custom_mode,
                    system_status: d.system_status as u8,
                    last_seen: now_ms(),
                });
            }
            mlink::MavMessage::ATTITUDE(d) => {
                self.attitude.roll = d.roll;
                self.attitude.pitch = d.pitch;
                self.attitude.yaw = d.yaw;
            }
            mlink::MavMessage::GLOBAL_POSITION_INT(d) => {
                self.gps.lat = d.lat as f64 / 1e7;
                self.gps.lon = d.lon as f64 / 1e7;
                self.gps.alt = d.alt as f32 / 1000.0;
                self.gps.relative_alt = d.relative_alt as f32 / 1000.0;
                self.gps.heading = d.hdg as f32;
                self.gps.seen = true;
            }
            mlink::MavMessage::SYS_STATUS(d) => {
                self.battery.voltage = d.voltage_battery as f32 / 1000.0;
                self.battery.current = d.current_battery as f32 / 100.0;
                self.battery.remaining_pct = Some(d.battery_remaining);
            }
            mlink::MavMessage::GPS_RAW_INT(d) => {
                self.gps.lat = d.lat as f64 / 1e7;
                self.gps.lon = d.lon as f64 / 1e7;
                self.gps.alt = d.alt as f32 / 1000.0;
                self.gps.fix_type = d.fix_type as u8;
                self.gps.seen = true;
                self.gps.satellites = d.satellites_visible;
                // eph/epv 单位 cm -> m；65535 = 未知
                if d.eph != 65535 {
                    self.gps.hdop = d.eph as f32 / 100.0;
                }
                if d.epv != 65535 {
                    self.gps.vdop = d.epv as f32 / 100.0;
                }
                if d.vel != 65535 {
                    self.gps.ground_speed = d.vel as f32 / 100.0;
                }
            }
            mlink::MavMessage::VFR_HUD(d) => {
                self.air.airspeed = d.airspeed;
                self.air.groundspeed = d.groundspeed;
                self.air.climb = d.climb;
                self.air.throttle = d.throttle;
            }
            mlink::MavMessage::RC_CHANNELS(d) => {
                self.rc.ch = [
                    d.chan1_raw, d.chan2_raw, d.chan3_raw, d.chan4_raw,
                    d.chan5_raw, d.chan6_raw, d.chan7_raw, d.chan8_raw,
                ];
                self.rc.chancount = d.chancount;
                self.rc.rssi = d.rssi;
                self.rc.seen = true;
                self.rc.last_seen = now_ms();
            }
            mlink::MavMessage::COMMAND_ACK(d) => {
                self.cmd_ack = Some(CommandAck {
                    command: d.command as u16,
                    result: d.result as u8,
                    ts: now_ms(),
                });
            }
            mlink::MavMessage::FENCE_STATUS(d) => {
                self.fence.seen = true;
                self.fence.breach_status = d.breach_status;
                self.fence.breach_count = d.breach_count;
                self.fence.breach_time = d.breach_time;
                self.fence.breach_type = d.breach_type as u8;
                self.fence.last_seen = now_ms();
            }
            _ => {}
        }
    }

    /// 是否武装（base_mode 的 MAV_MODE_FLAG_SAFETY_ARMED 位）
    pub fn is_armed(&self) -> bool {
        self.heartbeat
            .as_ref()
            .map(|h| h.base_mode & 0x80 != 0)
            .unwrap_or(false)
    }

    /// 最近一条指令回执的展示文本（含超时提示）
    pub fn ack_text(&self) -> Option<String> {
        let ack = self.cmd_ack.as_ref()?;
        let age_ms = now_ms().saturating_sub(ack.ts);
        let name = cmd_name(ack.command);
        if age_ms > 10_000 {
            Some(format!("{name}：已发送（超时无回执）"))
        } else {
            Some(format!("{name}：{}", ack.result_name()))
        }
    }

    /// 返回当前飞行模式的可读名称（基于 heartbeat 的 autopilot / custom_mode）。
    /// 已知映射：ArduPilot Copter / Plane / Rover / Sub、PX4、以及 MAV_MODE 回退。
    pub fn flight_mode_name(&self) -> String {
        let Some(hb) = &self.heartbeat else {
            return "未知".into();
        };
        let ap = hb.autopilot;
        let cm = hb.custom_mode;
        // 已知 autopilot 子协议号（MAV_AUTOPILOT）
        const ARDUPILOT: u8 = 3;
        const PX4: u8 = 12;
        if ap == ARDUPILOT {
            // ArduPilot 的 frame type 体现在 mav_type 上，模式表按机型略有不同，
            // 这里用最常用的 Copter 表 + 其余机型回退到 "模式 N"。
            copter_mode(cm)
        } else if ap == PX4 {
            px4_mode(cm)
        } else {
            format!("模式 {cm}")
        }
    }
}

/// ArduPilot Copter 自定义模式（详见 ArduCopter defines.h）
fn copter_mode(m: u32) -> String {
    let name = match m {
        0 => "STABILIZE",
        1 => "ACRO",
        2 => "ALT_HOLD",
        3 => "AUTO",
        4 => "GUIDED",
        5 => "LOITER",
        6 => "RTL",
        7 => "CIRCLE",
        9 => "LAND",
        11 => "DRIFT",
        13 => "SPORT",
        14 => "FLIP",
        15 => "AUTOTUNE",
        16 => "POSHOLD",
        17 => "BRAKE",
        18 => "THROW",
        19 => "AVOID_ADSB",
        20 => "GUIDED_NOGPS",
        21 => "SMART_RTL",
        22 => "FLOWHOLD",
        23 => "FOLLOW",
        24 => "ZIGZAG",
        25 => "SYSTEMID",
        26 => "AUTOROTATE",
        27 => "AUTO_RTL",
        _ => return format!("模式 {m}"),
    };
    name.to_string()
}

/// ArduPilot Plane 自定义模式（详见 ArduPlane defines.h）
fn plane_mode(m: u32) -> String {
    let name = match m {
        0 => "MANUAL",
        1 => "CIRCLE",
        2 => "STABILIZE",
        3 => "TRAINING",
        4 => "ACRO",
        5 => "FBWA",
        6 => "FBWB",
        7 => "CRUISE",
        8 => "AUTOTUNE",
        10 => "AUTO",
        11 => "RTL",
        12 => "LOITER",
        13 => "TAKEOFF",
        14 => "AVOID_ADSB",
        15 => "GUIDED",
        16 => "INITIALISING",
        17 => "QSTABILIZE",
        18 => "QHOVER",
        19 => "QLOITER",
        20 => "QLAND",
        21 => "QRTL",
        22 => "QAUTOTUNE",
        23 => "QACRO",
        24 => "THERMAL",
        _ => return format!("模式 {m}"),
    };
    name.to_string()
}

/// PX4 自定义模式（navigation mode 高 8 位，见 PX4 文档）
fn px4_mode(m: u32) -> String {
    let nav = (m >> 16) & 0xff;
    let name = match nav {
        0 => "MANUAL",
        1 => "ALTCTL",
        2 => "POSCTL",
        3 => "AUTO_MISSION",
        4 => "AUTO_LOITER",
        5 => "AUTO_RTL",
        6 => "AUTO_LAND",
        7 => "AUTO_RTGS",
        8 => "AUTO_FOLLOW",
        9 => "AUTO_PRECLAND",
        10 => "AUTO_VTOL_TAKEOFF",
        11 => "AUTO_VTOL_LAND",
        12 => "AUTO_TAKEOFF",
        13 => "AUTO_LANDENGFAIL",
        14 => "AUTO_LANDGPSFAIL",
        15 => "AUTO_DESCEND",
        16 => "TERMINATION",
        17 => "OFFBOARD",
        18 => "STABILIZED",
        19 => "AUTO_MC_TAKEOFF",
        _ => return format!("PX4 #{nav}"),
    };
    name.to_string()
}

#[allow(dead_code)]
fn _unused_plane(_m: u32) -> String {
    plane_mode(_m)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// MAV_CMD 编号 -> 可读名（常用飞行指令）
pub fn cmd_name(cmd: u16) -> String {
    let name = match cmd {
        16 => "导航航点",
        20 => "RTL",
        21 => "降落",
        22 => "起飞",
        176 => "设置模式",
        400 => "解锁/上锁",
        511 => "起飞(2)",
        300 => "预解锁",
        92 => "航向设定",
        _ => return format!("指令 {cmd}"),
    };
    name.to_string()
}
