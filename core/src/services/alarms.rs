//! 飞行告警监控：低电量、失控、地理围栏越界
//!
//! `FlightMonitor` 接收一个 `VehicleModel` 快照，按阈值评估并产出告警项。
//! 告警状态在内部去重，避免同一告警每帧重复触发。

use crate::vehicle::VehicleModel;
use serde::{Deserialize, Serialize};

/// 告警等级
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlarmLevel {
    Info,
    Warn,
    Critical,
}

/// 一条告警
#[derive(Debug, Clone, PartialEq)]
pub struct Alarm {
    pub level: AlarmLevel,
    pub code: &'static str,
    pub message: String,
}

/// 监控阈值配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    /// 电量低于该百分比触发告警
    pub battery_warn_pct: i8,
    pub battery_critical_pct: i8,
    /// 围栏半径（米），超过触发越界告警
    pub fence_radius_m: f64,
    /// 围栏中心（度）
    pub fence_lat: f64,
    pub fence_lon: f64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            battery_warn_pct: 30,
            battery_critical_pct: 15,
            fence_radius_m: 1000.0,
            fence_lat: 31.0,
            fence_lon: 121.0,
        }
    }
}

/// 飞行监控器
#[derive(Debug)]
pub struct FlightMonitor {
    cfg: MonitorConfig,
    /// 当前处于激活状态的告警 code 集合（去重用）
    active: std::collections::HashSet<&'static str>,
}

impl FlightMonitor {
    pub fn new(cfg: MonitorConfig) -> Self {
        Self {
            cfg,
            active: std::collections::HashSet::new(),
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(MonitorConfig::default())
    }

    /// 运行时更新监控阈值（用户编辑告警规则后调用）
    pub fn set_config(&mut self, cfg: MonitorConfig) {
        self.cfg = cfg;
    }

    /// 读取当前监控配置
    pub fn config(&self) -> &MonitorConfig {
        &self.cfg
    }

    /// 评估一次飞机状态，返回本次「新触发」的告警（已激活的不重复返回）
    pub fn evaluate(&mut self, v: &VehicleModel) -> Vec<Alarm> {
        let mut fired = Vec::new();

        // 1. 电量
        if let Some(rem) = v.battery.remaining_pct {
            if rem <= self.cfg.battery_critical_pct {
                fired.push(self.raise(
                    "BATT_CRIT",
                    AlarmLevel::Critical,
                    format!("电量极低：{rem}%"),
                ));
            } else if rem <= self.cfg.battery_warn_pct {
                fired.push(self.raise(
                    "BATT_LOW",
                    AlarmLevel::Warn,
                    format!("电量偏低：{rem}%"),
                ));
            } else {
                self.clear("BATT_CRIT");
                self.clear("BATT_LOW");
            }
        } else {
            self.clear("BATT_CRIT");
            self.clear("BATT_LOW");
        }

        // 2. 失联（心跳超时）
        if let Some(hb) = &v.heartbeat {
            let age = crate::vehicle::now_ms().saturating_sub(hb.last_seen);
            if age > 5000 {
                fired.push(self.raise(
                    "RC_LOST",
                    AlarmLevel::Critical,
                    format!("心跳超时：{age} ms 未收到"),
                ));
            } else {
                self.clear("RC_LOST");
            }
        }

        // 3. 地理围栏越界
        if v.gps.lat != 0.0 || v.gps.lon != 0.0 {
            let d = haversine_m(
                self.cfg.fence_lat,
                self.cfg.fence_lon,
                v.gps.lat,
                v.gps.lon,
            );
            if d > self.cfg.fence_radius_m {
                fired.push(self.raise(
                    "FENCE",
                    AlarmLevel::Warn,
                    format!("越界：距围栏中心 {d:.0} m（限 {} m）", self.cfg.fence_radius_m),
                ));
            } else {
                self.clear("FENCE");
            }
        }

        fired
    }

    /// 当前所有激活中的告警 code
    pub fn active_codes(&self) -> Vec<&'static str> {
        self.active.iter().copied().collect()
    }

    fn raise(&mut self, code: &'static str, level: AlarmLevel, message: String) -> Alarm {
        self.active.insert(code);
        Alarm {
            level,
            code,
            message,
        }
    }

    fn clear(&mut self, code: &'static str) {
        self.active.remove(code);
    }
}

/// 两点间大圆距离（米）
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let to_rad = std::f64::consts::PI / 180.0;
    let dlat = (lat2 - lat1) * to_rad;
    let dlon = (lon2 - lon1) * to_rad;
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_rad().cos() * lat2.to_rad().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    R * c
}

// f64 没有 to_rad，简单扩展
trait ToRad {
    fn to_rad(self) -> f64;
}
impl ToRad for f64 {
    fn to_rad(self) -> f64 {
        self * std::f64::consts::PI / 180.0
    }
}
