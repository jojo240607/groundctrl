//! 模拟链路（SITL / 回环测试）
//!
//! 向订阅者周期性推送一条合成心跳 + 姿态消息，用于无硬件时验证全链路。

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::{GcError, Result};
use crate::link::{Link, LinkQuality, LinkStats};
use crate::mlink;
use ::mavlink;

pub struct SimLink {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    stats: LinkStats,
}

impl SimLink {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let producer = tx.clone();
        tokio::spawn(async move {
            // 用 mavlink 编码合成消息
            let enc = |msg: mlink::MavMessage| -> Vec<u8> {
                let mut buf = Vec::new();
                if mlink::encode_v2(&mlink::default_header(), &msg).is_ok() {
                    buf
                } else {
                    Vec::new()
                }
            };
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(100));
            loop {
                tick.tick().await;
                let hb = mlink::MavMessage::HEARTBEAT(
                    ::mavlink::common::HEARTBEAT_DATA {
                        custom_mode: 0,
                        mavtype: ::mavlink::common::MavType::MAV_TYPE_QUADROTOR,
                        autopilot: ::mavlink::common::MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
                        base_mode: ::mavlink::common::MavModeFlag::MAV_MODE_FLAG_CUSTOM_MODE_ENABLED,
                        system_status:
                            ::mavlink::common::MavState::MAV_STATE_ACTIVE,
                        mavlink_version: 3,
                    },
                );
                let _ = producer.send(enc(hb));

                // 姿态：缓慢变化，模拟飞行
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();
                let att = mlink::MavMessage::ATTITUDE(
                    ::mavlink::common::ATTITUDE_DATA {
                        time_boot_ms: (t * 1000.0) as u32,
                        roll: (t * 0.5).sin() as f32 * 0.3,
                        pitch: (t * 0.3).cos() as f32 * 0.2,
                        yaw: (t * 0.1) as f32,
                        rollspeed: 0.0,
                        pitchspeed: 0.0,
                        yawspeed: 0.0,
                    },
                );
                let _ = producer.send(enc(att));

                let bat = mlink::MavMessage::SYS_STATUS(
                    ::mavlink::common::SYS_STATUS_DATA {
                        onboard_control_sensors_present:
                            ::mavlink::common::MavSysStatusSensor::empty(),
                        onboard_control_sensors_enabled:
                            ::mavlink::common::MavSysStatusSensor::empty(),
                        onboard_control_sensors_health:
                            ::mavlink::common::MavSysStatusSensor::empty(),
                        load: 100,
                        voltage_battery: 12000,
                        current_battery: -1,
                        battery_remaining: 80,
                        drop_rate_comm: 0,
                        errors_comm: 0,
                        errors_count1: 0,
                        errors_count2: 0,
                        errors_count3: 0,
                        errors_count4: 0,
                    },
                );
                let _ = producer.send(enc(bat));

                let gps = mlink::MavMessage::GLOBAL_POSITION_INT(
                    ::mavlink::common::GLOBAL_POSITION_INT_DATA {
                        time_boot_ms: (t * 1000.0) as u32,
                        lat: 311000000 + (t * 10.0) as i32,
                        lon: 121400000 + (t * 10.0) as i32,
                        alt: 10000,
                        relative_alt: 1000,
                        vx: 0,
                        vy: 0,
                        vz: 0,
                        hdg: (t * 10.0) as u16 % 360,
                    },
                );
                let _ = producer.send(enc(gps));
            }
        });
        Self {
            tx,
            rx: tokio::sync::Mutex::new(rx),
            stats: LinkStats::new(),
        }
    }
}

#[async_trait]
impl Link for SimLink {
    async fn send(&self, _bytes: &[u8]) -> Result<()> {
        Ok(()) // 模拟链路忽略下行
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
