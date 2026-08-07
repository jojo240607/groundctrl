//! 主应用结构：状态驱动、tokio runtime、链路连接与遥测订阅。

mod connect;
mod subscribe;

pub use connect::{ConnectKind, TabKind};
pub use subscribe::subscribe_telemetry;

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use groundctrl_core::link::{self, LinkHandle};
use groundctrl_core::services::TelemetryHub;

use crate::state::UiState;

/// 主应用
pub struct GroundControlApp {
    pub state: Arc<Mutex<UiState>>,
    pub hub: Arc<TelemetryHub>,
    pub rt: tokio::runtime::Runtime,
}

impl GroundControlApp {
    pub fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let hub = Arc::new(TelemetryHub::new());

        // 共享 UI 状态
        let state = Arc::new(Mutex::new(UiState::default()));

        // 订阅遥测与总线事件、周期刷新日志帧数
        subscribe_telemetry(&hub, &state, &rt);

        // 默认以 SimLink 启动，打开即有遥测数据
        {
            let hub = hub.clone();
            rt.spawn(async move {
                hub.connect(link::share(link::sim::SimLink::new())).await;
            });
        }

        Self { state, hub, rt }
    }

    /// 断开当前活动链路
    pub fn disconnect(&self) {
        let hub = self.hub.clone();
        let state = self.state.clone();
        self.rt.spawn(async move {
            hub.disconnect().await;
            if let Ok(mut s) = state.lock() {
                s.log.push("链路已断开".into());
            }
        });
    }

    /// 连接指定类型的链路（先断开旧链路，再设为当前活动链路）
    pub fn connect(&self, kind: ConnectKind) {
        let hub = self.hub.clone();
        let state = self.state.clone();
        self.rt.spawn(async move {
            let link: LinkHandle = match kind {
                ConnectKind::Sim => link::share(link::sim::SimLink::new()),
                ConnectKind::Udp(bind, target) => match link::udp::UdpLink::new(
                    link::udp::UdpConfig {
                        bind_addr: bind,
                        target_addr: target,
                    },
                )
                .await
                {
                    Ok(l) => link::share(l),
                    Err(e) => {
                        if let Ok(mut s) = state.lock() {
                            s.log.push(format!("UDP open failed: {e}"));
                        }
                        return;
                    }
                },
                ConnectKind::Serial(port, baud) => {
                    link::share(link::serial::SerialLink::new(link::serial::SerialConfig {
                        port,
                        baud_rate: baud,
                    }))
                }
            };
            hub.connect(link).await;
            if let Ok(mut s) = state.lock() {
                s.log.push("链路已连接".into());
            }
        });
    }

    /// 请求参数（飞控 sys=1, comp=1）
    pub fn request_params(&self) {
        let hub = self.hub.clone();
        self.rt.spawn(async move {
            if let Err(e) = hub.request_params(1, 1).await {
                tracing::warn!("request params failed: {e}");
            }
        });
    }

    /// 写回一个参数
    pub fn set_param(&self, name: String, value: f32) {
        let hub = self.hub.clone();
        self.rt.spawn(async move {
            let _ = hub.set_param(1, 1, &name, value).await;
        });
    }

    /// 上传本地航点（MISSION_COUNT + 逐条 ITEM）
    pub fn upload_mission(&self) {
        let hub = self.hub.clone();
        let state = self.state.clone();
        self.rt.spawn(async move {
            let mission = {
                if let Ok(s) = state.lock() {
                    s.mission.clone()
                } else {
                    return;
                }
            };
            if mission.is_empty() {
                return;
            }
            if let Err(e) = hub.upload_mission(1, 1, &mission).await {
                tracing::warn!("upload mission failed: {e}");
            }
        });
    }
}

/// 默认刷新间隔（用于 request_repaint_after）
pub const REPAINT_INTERVAL: Duration = Duration::from_millis(50);

/// 当前 Unix 秒（用于告警时间戳）
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
