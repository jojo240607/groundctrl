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
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Battery {
    pub voltage: f32,        // V
    pub current: f32,        // A
    /// 剩余电量百分比（None = 未知 / 飞控未上报）
    pub remaining_pct: Option<i8>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HeartbeatInfo {
    pub system_id: u8,
    pub component_id: u8,
    pub mav_type: u8,
    pub autopilot: u8,
    pub base_mode: u8,
    pub system_status: u8,
    pub last_seen: u64, // ms 时间戳
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
                self.gps.satellites = d.satellites_visible;
            }
            _ => {}
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
