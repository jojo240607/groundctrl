//! 前端(Web)与后端(Rust)之间的序列化数据契约。
//!
//! 这些类型把 `groundctrl-core` 的内部模型转换成前端 JS/JSON 友好的形状。
//! 这样 core 无需为其内部类型强行实现 `Serialize`，也避免把 mavlink 原始头暴露给前端。

use groundctrl_core::services::alarms::{AlarmLevel, MonitorConfig};
use groundctrl_core::vehicle::VehicleModel;
use serde::Serialize;

/// 单个飞行器（机队中的一架）的遥测快照。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VehicleSnapshot {
    pub sysid: u8,
    pub compid: u8,
    pub name: String,
    pub connected: bool,
    pub flight_mode: String,
    pub armed: Option<bool>,
    pub battery: Option<f32>,
    pub voltage: Option<f32>,
    pub current: Option<f32>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub alt_rel: Option<f32>,
    pub alt_abs: Option<f32>,
    pub relative_alt: Option<f32>,
    pub vx: Option<f32>,
    pub vy: Option<f32>,
    pub vz: Option<f32>,
    pub roll: Option<f32>,
    pub pitch: Option<f32>,
    pub yaw: Option<f32>,
    pub ground_speed: Option<f32>,
    pub air_speed: Option<f32>,
    pub heading: Option<f32>,
    pub gps_fix: Option<u8>,
    pub satellites: Option<u8>,
    pub hdop: Option<f32>,
    pub last_update: i64,
}

impl VehicleSnapshot {
    pub fn from_vehicle(v: &VehicleModel) -> Self {
        // MAV_MODE_FLAG 中 bit7 (0x80) 表示已解锁(armed)
        const MAV_MODE_FLAG_SAFETY_ARMED: u8 = 0x80;
        let armed = v
            .heartbeat
            .as_ref()
            .map(|h| (h.base_mode & MAV_MODE_FLAG_SAFETY_ARMED) != 0);
        let battery = v.battery.remaining_pct.filter(|p| *p >= 0).map(|p| p as f32);
        VehicleSnapshot {
            sysid: v.sys_id,
            compid: v.comp_id,
            name: v.link_name.clone(),
            connected: v.online,
            flight_mode: v.flight_mode_name(),
            armed,
            battery,
            voltage: Some(v.battery.voltage),
            current: Some(v.battery.current),
            lat: Some(v.gps.lat),
            lon: Some(v.gps.lon),
            alt_rel: Some(v.gps.relative_alt),
            alt_abs: Some(v.gps.alt),
            relative_alt: Some(v.gps.relative_alt),
            vx: None,
            vy: None,
            vz: Some(v.air.climb),
            roll: Some(v.attitude.roll),
            pitch: Some(v.attitude.pitch),
            yaw: Some(v.attitude.yaw),
            ground_speed: Some(v.air.groundspeed),
            air_speed: Some(v.air.airspeed),
            heading: Some(v.gps.heading),
            gps_fix: Some(v.gps.fix_type),
            satellites: Some(v.gps.satellites),
            hdop: None,
            last_update: v.heartbeat.as_ref().map(|h| h.last_seen as i64).unwrap_or(0),
        }
    }
}

/// 机队快照（所有已发现飞行器）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetSnapshot {
    pub vehicles: Vec<VehicleSnapshot>,
    /// 当前选中系统的 sysid，无则 0。
    pub selected: u8,
}

/// 告警级别。
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AlarmLevelJson {
    Info,
    Warn,
    Critical,
}

/// 告警事件（推送给前端）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmJson {
    /// 来源链路名（core 未带 sysid，这里用链路标识）
    pub link: String,
    pub code: String,
    pub level: AlarmLevelJson,
    pub message: String,
    pub timestamp_ms: u64,
}

/// 链路状态事件（推送给前端）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStateJson {
    pub link: String,
    pub connected: bool,
}

/// 参数表条目（用于前端参数面板）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamItem {
    pub index: u16,
    pub name: String,
    pub value: f32,
}

/// 航点条目。
#[derive(Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaypointItem {
    pub seq: u16,
    pub command: u16,
    pub x: f64,
    pub y: f64,
    pub z: f32,
    pub autocontinue: bool,
}

/// 告警规则配置（前端编辑 -> 后端写入 core）。
#[derive(Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorConfigJson {
    pub battery_warn_pct: i8,
    pub battery_critical_pct: i8,
    pub fence_radius_m: f64,
    pub fence_lat: f64,
    pub fence_lon: f64,
}

impl From<MonitorConfig> for MonitorConfigJson {
    fn from(c: MonitorConfig) -> Self {
        MonitorConfigJson {
            battery_warn_pct: c.battery_warn_pct,
            battery_critical_pct: c.battery_critical_pct,
            fence_radius_m: c.fence_radius_m,
            fence_lat: c.fence_lat,
            fence_lon: c.fence_lon,
        }
    }
}

impl From<MonitorConfigJson> for MonitorConfig {
    fn from(c: MonitorConfigJson) -> Self {
        MonitorConfig {
            battery_warn_pct: c.battery_warn_pct,
            battery_critical_pct: c.battery_critical_pct,
            fence_radius_m: c.fence_radius_m,
            fence_lat: c.fence_lat,
            fence_lon: c.fence_lon,
        }
    }
}

/// 设置（持久化 + 前端读写）。
#[derive(Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsJson {
    pub default_url: String,
    pub trend_enabled: bool,
    pub trend_selected: Vec<String>,
}

impl Default for SettingsJson {
    fn default() -> Self {
        SettingsJson {
            default_url: "tcp:127.0.0.1:5760".to_string(),
            trend_enabled: false,
            trend_selected: Vec::new(),
        }
    }
}
