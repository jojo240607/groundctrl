//! 主应用结构：状态驱动、tokio runtime、链路连接与遥测订阅。

mod connect;
mod settings;
mod subscribe;

pub use connect::{ConnectKind, TabKind};
pub use settings::AppSettings;
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
    /// 当前加载的设置（用于保存时回写）
    settings: Arc<Mutex<AppSettings>>,
}

impl GroundControlApp {
    pub fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let hub = Arc::new(TelemetryHub::new());

        // 加载持久化设置
        let settings = Arc::new(Mutex::new(AppSettings::load()));

        // 共享 UI 状态：用设置初始化连接/地图默认值
        let mut ui_state = UiState::default();
        {
            let s = settings.lock().unwrap();
            ui_state.serial_port = s.serial_port.clone();
            ui_state.baud = s.baud;
            ui_state.udp_bind = s.udp_bind.clone();
            ui_state.udp_target = s.udp_target.clone();
            ui_state.tile_dir = s.tile_dir.clone();
            ui_state.map_zoom = s.map_zoom;
        }
        let state = Arc::new(Mutex::new(ui_state));

        // 订阅遥测与总线事件、周期刷新日志帧数
        subscribe_telemetry(&hub, &state, &rt);

        // 默认以 SimLink 启动，打开即有遥测数据
        {
            let hub = hub.clone();
            rt.spawn(async move {
                hub.connect(link::share(link::sim::SimLink::new())).await;
            });
        }

        Self {
            state,
            hub,
            rt,
            settings,
        }
    }

    /// 轻量保存句柄（可 Clone，供异步闭包中调用，避免持有整个 App）
    pub fn save_handle(&self) -> SaveHandle {
        SaveHandle {
            state: self.state.clone(),
            settings: self.settings.clone(),
        }
    }

    /// 将当前 UI 状态回写到设置并保存到磁盘。
    /// 在连接配置变更、地图目录选择、退出时调用。
    pub fn save_settings(&self) {
        self.save_handle().save();
    }

    /// 记录当前窗口尺寸到设置（每帧调用，纯内存写入，落盘在 save 时）
    pub fn record_window_size(&self, w: f32, h: f32) {
        if let Ok(mut s) = self.settings.lock() {
            s.window_w = w;
            s.window_h = h;
        }
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

/// 轻量保存句柄：持有 state 与 settings 的 Arc，可在异步闭包中安全调用 `save`。
/// 避免把整个 `GroundControlApp`（含 tokio Runtime，不可 Clone）传入闭包。
#[derive(Clone)]
pub struct SaveHandle {
    state: Arc<Mutex<UiState>>,
    settings: Arc<Mutex<AppSettings>>,
}

impl SaveHandle {
    /// 从 UI 状态读取连接/地图偏好，写入 settings 并落盘。
    pub fn save(&self) {
        let mut s = self.settings.lock().unwrap();
        if let Ok(st) = self.state.lock() {
            s.serial_port = st.serial_port.clone();
            s.baud = st.baud;
            s.udp_bind = st.udp_bind.clone();
            s.udp_target = st.udp_target.clone();
            s.tile_dir = st.tile_dir.clone();
            s.map_zoom = st.map_zoom;
        }
        s.save();
    }
}
