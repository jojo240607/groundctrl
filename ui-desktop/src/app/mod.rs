//! 主应用结构：状态驱动、tokio runtime、链路连接与遥测订阅。

mod connect;
mod settings;
mod sound;
mod subscribe;

pub use connect::{ConnectKind, TabKind};
pub use settings::AppSettings;
pub use subscribe::subscribe_telemetry;

/// 快捷飞行指令（对应 TelemetryHub 的 COMMAND_LONG 封装）
#[derive(Debug, Clone, Copy)]
pub enum FlightCommand {
    Arm,
    Disarm,
    /// 一键起飞（目标高度米）
    Takeoff(f32),
    Land,
    Rtl,
    /// 切换飞行模式（custom_mode，ArduPilot Copter 编号）
    SetMode(u32),
}

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
    /// 游戏手柄（P2-3 摇杆操控）；无手柄驱动时为 None
    joystick: Option<gilrs::Gilrs>,
    /// 声音告警播放器（P2-4）；无音频设备时为 None（静默）
    sound: Option<sound::Sounder>,
    /// 视频接收器与模拟源任务（P3-2）
    pub video: Arc<Mutex<VideoCtrl>>,
    /// 脚本任务（P3-4）
    pub scripts: Arc<Mutex<ScriptCtrl>>,
}

/// 视频控制（P3-2）：UDP MJPEG 接收器 + 模拟视频源任务
pub struct VideoCtrl {
    /// 接收器（None = 未启动）
    pub rx: Option<groundctrl_core::services::video::VideoReceiver>,
    /// 模拟视频源任务句柄（None = 未运行）
    pub sim: Option<tokio::task::JoinHandle<()>>,
}

impl VideoCtrl {
    fn new() -> Self {
        Self { rx: None, sim: None }
    }
}

/// 脚本控制（P3-4）：运行任务句柄 + 中止信号
pub struct ScriptCtrl {
    /// 脚本运行任务（None = 未运行）
    pub task: Option<tokio::task::JoinHandle<()>>,
    /// 中止信号（每次运行新建，旧任务持有旧信号，互不干扰）
    pub abort: groundctrl_core::services::script::ScriptAbort,
}

impl ScriptCtrl {
    fn new() -> Self {
        Self {
            task: None,
            abort: groundctrl_core::services::script::ScriptAbort::new(),
        }
    }
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
            ui_state.online_tiles = s.online_tiles;
            ui_state.tile_url = s.tile_url.clone();
            ui_state.map_zoom = s.map_zoom;
            ui_state.trend_selected = s.trend_selected.clone();
            ui_state.trend_enabled = s.trend_enabled;
            ui_state.lang = s.lang;
        }
        ui_state.sound_enabled = true; // 声音告警默认开启
        ui_state.video_addr = "0.0.0.0:5600".into(); // 视频默认监听地址
        ui_state.video_osd = true; // OSD 默认开启
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
            joystick: gilrs::Gilrs::new().ok(),
            sound: sound::Sounder::new(),
            video: Arc::new(Mutex::new(VideoCtrl::new())),
            scripts: Arc::new(Mutex::new(ScriptCtrl::new())),
        }
    }

    /// 检测新增告警并播放提示音（每帧调用）
    pub fn play_alarm_sounds(&self) {
        let Some(snd) = &self.sound else {
            return;
        };
        let Ok(mut s) = self.state.lock() else {
            return;
        };
        if !s.sound_enabled {
            return;
        }
        // alarms 只追加；记录已播放条数，新条目按序播报
        let total = s.alarms.len();
        let mut played = s.sound_alarm_played;
        while played < total {
            if let Some((_, a)) = s.alarms.get(played) {
                snd.alarm(a.level);
            }
            played += 1;
        }
        s.sound_alarm_played = played;
    }

    /// 轮询手柄事件与轴状态（每帧调用，结果写入共享状态）
    pub fn poll_joystick(&mut self) {
        let Some(gilrs) = self.joystick.as_mut() else {
            return;
        };
        while gilrs.next_event().is_some() {}
        let mut name = String::new();
        let mut axes = [0.0f32; 6];
        for (_, gp) in gilrs.gamepads() {
            name = gp.name().to_string();
            axes[0] = gp.value(gilrs::Axis::LeftStickX);
            axes[1] = gp.value(gilrs::Axis::LeftStickY);
            axes[2] = gp.value(gilrs::Axis::RightStickX);
            axes[3] = gp.value(gilrs::Axis::RightStickY);
            axes[4] = gp.value(gilrs::Axis::LeftZ);
            axes[5] = gp.value(gilrs::Axis::RightZ);
            break; // 只读取第一个手柄
        }
        if let Ok(mut s) = self.state.lock() {
            s.joystick_name = name;
            s.joystick_axes = axes;
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

    /// 发送飞行指令（解锁/上锁/起飞/降落/返航）
    pub fn send_command(&self, kind: FlightCommand) {
        let hub = self.hub.clone();
        self.rt.spawn(async move {
            let r = match kind {
                FlightCommand::Arm => hub.send_arm_disarm(1, 1, true).await,
                FlightCommand::Disarm => hub.send_arm_disarm(1, 1, false).await,
                FlightCommand::Takeoff(alt) => hub.send_takeoff(1, 1, alt).await,
                FlightCommand::Land => hub.send_land(1, 1).await,
                FlightCommand::Rtl => hub.send_rtl(1, 1).await,
                FlightCommand::SetMode(m) => hub.send_mode(1, 1, m).await,
            };
            if let Err(e) = r {
                tracing::warn!("send command failed: {e}");
            }
        });
    }

    /// 请求飞控按流率上报数据（REQUEST_DATA_STREAM）
    pub fn request_stream(&self, stream_id: u8, rate_hz: u16) {
        let hub = self.hub.clone();
        self.rt.spawn(async move {
            if let Err(e) = hub.request_data_stream_by_id(1, 1, stream_id, rate_hz).await {
                tracing::warn!("request stream failed: {e}");
            }
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

    /// 上传地理围栏到飞控
    pub fn upload_fence(&self) {
        let hub = self.hub.clone();
        let state = self.state.clone();
        self.rt.spawn(async move {
            let pts = {
                if let Ok(s) = state.lock() {
                    s.fence.clone()
                } else {
                    return;
                }
            };
            if pts.len() < 3 {
                if let Ok(mut s) = state.lock() {
                    s.fence_msg = "围栏至少需要 3 个点".into();
                }
                return;
            }
            match hub.upload_fence(1, 1, &pts).await {
                Ok(_) => {
                    if let Ok(mut s) = state.lock() {
                        s.fence_msg = format!("围栏已上传（{} 个点）", pts.len());
                    }
                }
                Err(e) => {
                    if let Ok(mut s) = state.lock() {
                        s.fence_msg = format!("上传失败: {e}");
                    }
                }
            }
        });
    }

    /// 从飞控下载地理围栏
    pub fn download_fence(&self) {
        let hub = self.hub.clone();
        let state = self.state.clone();
        self.rt.spawn(async move {
            match hub.download_fence(1, 1).await {
                Ok(pts) => {
                    if let Ok(mut s) = state.lock() {
                        s.fence = pts.clone();
                        s.fence_msg = format!("已下载（{} 个点）", pts.len());
                    }
                }
                Err(e) => {
                    if let Ok(mut s) = state.lock() {
                        s.fence_msg = format!("下载失败: {e}");
                    }
                }
            }
        });
    }

    // ---------- 视频 / OSD（P3-2） ----------

    /// 启动 UDP MJPEG 视频接收
    pub fn start_video(&self, addr: String) {
        let video = self.video.clone();
        let st = self.state.clone();
        self.rt.spawn(async move {
            match groundctrl_core::services::video::VideoReceiver::start(&addr).await {
                Ok(rx) => {
                    video.lock().unwrap().rx = Some(rx);
                    if let Ok(mut s) = st.lock() {
                        s.video_running = true;
                        s.video_msg = format!("✅ 视频接收已启动: {addr}");
                    }
                }
                Err(e) => {
                    if let Ok(mut s) = st.lock() {
                        s.video_msg = format!("视频启动失败: {e}");
                    }
                }
            }
        });
    }

    /// 停止视频接收（保留最后一帧画面）
    pub fn stop_video(&self) {
        let mut ctrl = self.video.lock().unwrap();
        if let Some(rx) = ctrl.rx.take() {
            rx.stop();
        }
        drop(ctrl);
        if let Ok(mut s) = self.state.lock() {
            s.video_running = false;
            s.video_msg = "视频接收已停止".into();
        }
    }

    /// 启动模拟视频源：本机生成合成 JPEG 帧，发往监听地址（无需摄像头）
    pub fn start_video_sim(&self) {
        // 先停掉旧的模拟源
        let mut ctrl = self.video.lock().unwrap();
        if let Some(h) = ctrl.sim.take() {
            h.abort();
        }
        drop(ctrl);
        // 从监听地址解析端口（默认 5600）
        let port: u16 = {
            let s = self.state.lock().unwrap();
            s.video_addr
                .rsplit(':')
                .next()
                .and_then(|p| p.parse().ok())
                .unwrap_or(5600)
        };
        let st = self.state.clone();
        let handle = self.rt.spawn(async move {
            let target = format!("127.0.0.1:{port}");
            let Ok(sock) = tokio::net::UdpSocket::bind("127.0.0.1:0").await else {
                return;
            };
            if sock.connect(&target).await.is_err() {
                if let Ok(mut s) = st.lock() {
                    s.video_msg = format!("模拟源无法发送到 {target}");
                }
                return;
            }
            if let Ok(mut s) = st.lock() {
                s.video_sim = true;
                s.video_msg = format!("模拟视频源运行中 → {target}");
            }
            // 10fps，最多 600 帧（60 秒）
            for i in 0..600u32 {
                let frame = sim_jpeg_frame(i);
                if sock.send(&frame).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            if let Ok(mut s) = st.lock() {
                s.video_sim = false;
                s.video_msg = "模拟视频源已结束".into();
            }
        });
        self.video.lock().unwrap().sim = Some(handle);
    }

    /// 停止模拟视频源
    pub fn stop_video_sim(&self) {
        let mut ctrl = self.video.lock().unwrap();
        if let Some(h) = ctrl.sim.take() {
            h.abort();
        }
        drop(ctrl);
        if let Ok(mut s) = self.state.lock() {
            s.video_sim = false;
            s.video_msg = "模拟视频源已停止".into();
        }
    }

    // ---------- 脚本任务（P3-4） ----------

    /// 解析并运行脚本任务（先终止旧任务，避免并发执行）
    pub fn run_script(&self) {
        use groundctrl_core::services::script::{parse_script, run_script, ScriptAbort, ScriptEvent};

        // 取出编辑内容并准备新中止信号
        let (text, name) = {
            let s = self.state.lock().unwrap();
            (s.script_text.clone(), s.script_name.clone())
        };
        let hub = self.hub.clone();
        let st = self.state.clone();
        {
            let mut ctrl = self.scripts.lock().unwrap();
            if let Some(h) = ctrl.task.take() {
                h.abort();
            }
            ctrl.abort = ScriptAbort::new();
        }
        let abort = self.scripts.lock().unwrap().abort.clone();

        let handle = self.rt.spawn(async move {
            // 1) 解析
            let prog = match parse_script(&name, &text) {
                Ok(p) => p,
                Err(e) => {
                    if let Ok(mut s) = st.lock() {
                        s.script_running = false;
                        s.script_msg = format!("解析失败: {e}");
                    }
                    return;
                }
            };
            if let Ok(mut s) = st.lock() {
                s.script_running = true;
                s.script_log.clear();
                s.script_msg = "运行中...".into();
            }
            // 2) 执行（条件判断取当前机队快照）
            let mut on_event = |ev: ScriptEvent| {
                if let Ok(mut s) = st.lock() {
                    match ev {
                        ScriptEvent::Step(i, d) => s.script_log.push(format!("[{i}] {d}")),
                        ScriptEvent::Log(t) => s.script_log.push(t),
                    }
                    if s.script_log.len() > 500 {
                        let n = s.script_log.len() - 500;
                        s.script_log.drain(0..n);
                    }
                }
            };
            let get_vehicle = || st.lock().ok().map(|s| s.vehicle.clone());
            let r = run_script(&hub, get_vehicle, &prog, &abort, &mut on_event).await;
            // 3) 结束反馈
            if let Ok(mut s) = st.lock() {
                s.script_running = false;
                s.script_msg = match r {
                    Ok(_) => "✅ 脚本执行完成".into(),
                    Err(e) => format!("脚本中止: {e}"),
                };
            }
        });
        self.scripts.lock().unwrap().task = Some(handle);
    }

    /// 停止脚本任务（置中止信号 + 强制取消任务）
    pub fn stop_script(&self) {
        let mut ctrl = self.scripts.lock().unwrap();
        ctrl.abort.abort();
        if let Some(h) = ctrl.task.take() {
            h.abort();
        }
        drop(ctrl);
        if let Ok(mut s) = self.state.lock() {
            s.script_running = false;
            s.script_msg = "已停止".into();
        }
    }
}

/// 生成一帧合成 JPEG（模拟图传源）：背景色循环 + 移动白色方块
fn sim_jpeg_frame(idx: u32) -> Vec<u8> {
    const W: u32 = 160;
    const H: u32 = 120;
    let mut img = image::RgbaImage::new(W, H);
    let phase = (idx % 120) as f32 / 120.0;
    let (r, g, b) = hsl_to_rgb(phase * 360.0, 0.7, 0.45);
    for (_, _, px) in img.enumerate_pixels_mut() {
        *px = image::Rgba([r, g, b, 255]);
    }
    // 白色方块沿对角移动
    let sq = 24u32;
    let mx = ((idx as i32 * 3) % (W as i32 - sq as i32)).max(0) as u32;
    let my = ((idx as i32 * 2) % (H as i32 - sq as i32)).max(0) as u32;
    for y in my..(my + sq).min(H) {
        for x in mx..(mx + sq).min(W) {
            img.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
        }
    }
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 70);
    let _ = enc.encode_image(&img);
    buf
}

/// HSL -> RGB（0..1 输入，255 输出）
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (
        ((r1 + m) * 255.0).clamp(0.0, 255.0) as u8,
        ((g1 + m) * 255.0).clamp(0.0, 255.0) as u8,
        ((b1 + m) * 255.0).clamp(0.0, 255.0) as u8,
    )
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
            s.online_tiles = st.online_tiles;
            s.tile_url = st.tile_url.clone();
            s.map_zoom = st.map_zoom;
            s.trend_selected = st.trend_selected.clone();
            s.trend_enabled = st.trend_enabled;
            s.lang = st.lang;
        }
        s.save();
    }
}
