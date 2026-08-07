//! 模拟链路（SITL / 回环测试）
//!
//! 向订阅者周期性推送合成心跳 + 姿态 + 电量 + GPS 消息，用于无硬件时验证全链路。
//! 同时响应 GCS 下行：
//! - 收到 `PARAM_REQUEST_LIST` 时，回放一组示例 `PARAM_VALUE`
//! - 电量随时间缓慢下降，用于验证告警逻辑

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};

use ::mavlink::common as mav;
use crate::error::{GcError, Result};
use crate::link::{Link, LinkQuality, LinkStats};
use crate::mlink;
use crate::vehicle::params::string_to_cstr;

/// 模拟链路内部可变状态
struct SimState {
    /// 电量百分比（随时间下降）
    battery_pct: i8,
}

impl SimState {
    fn new() -> Self {
        Self { battery_pct: 80 }
    }
}

pub struct SimLink {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    stats: LinkStats,
    header: ::mavlink::MavHeader,
}

impl SimLink {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let producer = tx.clone();
        let state = Arc::new(Mutex::new(SimState::new()));
        let st = state.clone();

        // 模拟飞控的 system/component id
        let header = ::mavlink::MavHeader {
            system_id: 1,
            component_id: 1,
            sequence: 0,
        };

        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        tokio::spawn(async move {
            let enc = |msg: mlink::MavMessage| -> Vec<u8> {
                match mlink::encode_v2(&header, &msg) {
                    Ok(b) => b,
                    Err(_) => Vec::new(),
                }
            };

            let mut tick = tokio::time::interval(Duration::from_millis(100));
            loop {
                tick.tick().await;
                let t = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();
                // 相对启动时刻的运行秒数（用于平滑变化）
                let elapsed_sec = t - start;

                // 电量每 ~2s 降 1%（用取模把范围限制在 [5,80]，避免溢出）
                {
                    let mut s = st.lock().await;
                    let drop = ((elapsed_sec as i32) / 2) % 80; // 每 2s 计 1, 最多降 80
                    let new_pct = 80 - drop;
                    s.battery_pct = if new_pct < 5 { 5 } else { new_pct as i8 };
                }

                let hb = mlink::MavMessage::HEARTBEAT(mav::HEARTBEAT_DATA {
                    custom_mode: 0,
                    mavtype: mav::MavType::MAV_TYPE_QUADROTOR,
                    autopilot: mav::MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
                    base_mode: mav::MavModeFlag::MAV_MODE_FLAG_CUSTOM_MODE_ENABLED,
                    system_status: mav::MavState::MAV_STATE_ACTIVE,
                    mavlink_version: 3,
                });
                let _ = producer.send(enc(hb));

                let att = mlink::MavMessage::ATTITUDE(mav::ATTITUDE_DATA {
                    time_boot_ms: (t * 1000.0) as u32,
                    roll: (t * 0.5).sin() as f32 * 0.3,
                    pitch: (t * 0.3).cos() as f32 * 0.2,
                    yaw: (t * 0.1) as f32,
                    rollspeed: 0.0,
                    pitchspeed: 0.0,
                    yawspeed: 0.0,
                });
                let _ = producer.send(enc(att));

                let battery_pct = st.lock().await.battery_pct;
                let bat = mlink::MavMessage::SYS_STATUS(mav::SYS_STATUS_DATA {
                    onboard_control_sensors_present: mav::MavSysStatusSensor::empty(),
                    onboard_control_sensors_enabled: mav::MavSysStatusSensor::empty(),
                    onboard_control_sensors_health: mav::MavSysStatusSensor::empty(),
                    load: 100,
                    voltage_battery: 12000,
                    current_battery: -1,
                    battery_remaining: battery_pct,
                    drop_rate_comm: 0,
                    errors_comm: 0,
                    errors_count1: 0,
                    errors_count2: 0,
                    errors_count3: 0,
                    errors_count4: 0,
                });
                let _ = producer.send(enc(bat));

                let gps = mlink::MavMessage::GLOBAL_POSITION_INT(mav::GLOBAL_POSITION_INT_DATA {
                    time_boot_ms: (t * 1000.0) as u32,
                    lat: 311_000_000 + ((t * 10.0) as i32 % 100_000),
                    lon: 121_400_000 + ((t * 10.0) as i32 % 100_000),
                    alt: 10000,
                    relative_alt: 1000,
                    vx: 0,
                    vy: 0,
                    vz: 0,
                    hdg: (t * 10.0) as u16 % 360,
                });
                let _ = producer.send(enc(gps));

                // 空速 / 地速 / 垂直速度 / 油门（合成，用于 HUD 仪表验证）
                let vfr = mlink::MavMessage::VFR_HUD(mav::VFR_HUD_DATA {
                    airspeed: 15.0 + (t * 0.7).sin() as f32 * 4.0,
                    groundspeed: 14.0 + (t * 0.6).cos() as f32 * 3.0,
                    heading: (t * 10.0) as i16 % 360,
                    throttle: (50.0 + (t * 0.9).sin() as f32 * 20.0) as u16,
                    alt: 10.0,
                    climb: (t * 0.4).sin() as f32 * 2.0,
                });
                let _ = producer.send(enc(vfr));
            }
        });

        Self {
            tx,
            rx: tokio::sync::Mutex::new(rx),
            stats: LinkStats::new(),
            header,
        }
    }

    /// 响应 GCS 下行：收到 PARAM_REQUEST_LIST 后回放示例参数
    async fn on_downlink(&self, bytes: &[u8]) {
        let mut cur = std::io::Cursor::new(bytes);
        let parsed = ::mavlink::read_v2_msg::<mlink::MavMessage, _>(&mut cur);
        if let Ok((_h, msg)) = parsed {
            if let mav::MavMessage::PARAM_REQUEST_LIST(_) = &msg {
                // 回放一组示例参数
                let params: &[(&str, f32)] = &[
                    ("SYSID_THISMAV", 1.0),
                    ("RTL_ALT", 15.0),
                    ("FS_THR_ENABLE", 1.0),
                    ("WPNAV_SPEED", 500.0),
                    ("BATT_CAPACITY", 3300.0),
                ];
                let count = params.len() as u16;
                for (i, (name, val)) in params.iter().enumerate() {
                    let pv = mav::PARAM_VALUE_DATA {
                        param_id: string_to_cstr(name),
                        param_value: *val,
                        param_type: mav::MavParamType::MAV_PARAM_TYPE_REAL32,
                        param_count: count,
                        param_index: i as u16,
                    };
                    if let Ok(b) = mlink::encode_v2(&self.header, &mlink::MavMessage::PARAM_VALUE(pv))
                    {
                        let _ = self.tx.send(b);
                    }
                }
            }
        }
    }
}

impl Default for SimLink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Link for SimLink {
    async fn send(&self, bytes: &[u8]) -> Result<()> {
        // 模拟链路：把下行交给内部逻辑（如参数请求应答）
        self.on_downlink(bytes).await;
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(b) => {
                self.stats.add_recv(b.len() as u64).await;
                Ok(b)
            }
            None => Err(GcError::Link("sim closed".into())),
        }
    }

    fn quality(&self) -> LinkQuality {
        LinkQuality {
            signal_pct: 100,
            ..LinkQuality::default()
        }
    }

    fn is_open(&self) -> bool {
        true
    }

    fn name(&self) -> String {
        "sim".into()
    }
}
