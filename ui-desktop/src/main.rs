//! 飞控地面站 — 桌面端（egui / eframe）
//!
//! 启动 tokio runtime 驱动 core 的 TelemetryHub，egui 订阅遥测快照并显示。

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use groundctrl_core::link::{self, LinkHandle};
use groundctrl_core::services::alarms::{Alarm, AlarmLevel};
use groundctrl_core::services::TelemetryHub;
use groundctrl_core::vehicle::params::ParamEntry;
use groundctrl_core::vehicle::mission::Waypoint;
use groundctrl_core::vehicle::VehicleModel;

/// 面板标签页
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TabKind {
    #[default]
    Telemetry,
    Params,
    Mission,
    Map,
    Log,
}

/// UI 共享状态
#[derive(Default)]
struct UiState {
    /// 当前遥测快照（最新一架飞机）
    vehicle: VehicleModel,
    log: Vec<String>,
    link_status: String,
    /// 当前活动链路名称（None = 未连接）
    active_link: Option<String>,
    /// 连接配置
    serial_port: String,
    baud: u32,
    udp_bind: String,
    udp_target: String,
    /// 当前标签页
    tab: TabKind,
    /// 参数缓存
    params: Vec<ParamEntry>,
    params_complete: bool,
    params_received: u16,
    params_expected: u16,
    /// 告警（累积，带时间戳）
    alarms: Vec<(u64, Alarm)>,
    /// GPS 轨迹历史（经纬度）
    trail: Vec<(f64, f64)>,
    /// 日志帧数
    log_frames: usize,
    /// 本地航点编辑
    mission: Vec<Waypoint>,
    wp_lat: f64,
    wp_lon: f64,
    wp_alt: f32,
}

struct GroundControlApp {
    state: Arc<Mutex<UiState>>,
    hub: Arc<TelemetryHub>,
    rt: tokio::runtime::Runtime,
}

impl GroundControlApp {
    fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        let hub = Arc::new(TelemetryHub::new());

        // 订阅遥测，写入共享状态
        let state = Arc::new(Mutex::new(UiState::default()));
        {
            let hub = hub.clone();
            let state = state.clone();
            rt.spawn(async move {
                let mut rx = hub.subscribe_telemetry();
                loop {
                    match rx.recv().await {
                        Ok(vm) => {
                            if let Ok(mut s) = state.lock() {
                                s.vehicle = vm.clone();
                                // 记录 GPS 轨迹
                                if vm.gps.lat != 0.0 || vm.gps.lon != 0.0 {
                                    if s.trail.last() != Some(&(vm.gps.lat, vm.gps.lon)) {
                                        s.trail.push((vm.gps.lat, vm.gps.lon));
                                        if s.trail.len() > 2000 {
                                            s.trail.remove(0);
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        // 订阅总线事件（链路状态 / 参数 / 告警）
        {
            let hub = hub.clone();
            let state = state.clone();
            rt.spawn(async move {
                let mut rx = hub.bus().subscribe();
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            match ev {
                                groundctrl_core::proto::bus::BusEvent::LinkState { link, open } => {
                                    if let Ok(mut s) = state.lock() {
                                        if open {
                                            s.active_link = Some(link);
                                        } else if s.active_link.as_deref() == Some(link.as_str()) {
                                            s.active_link = None;
                                        }
                                        s.link_status = match &s.active_link {
                                            Some(n) => format!("{n}: OPEN"),
                                            None => "无连接".to_string(),
                                        };
                                    }
                                }
                                groundctrl_core::proto::bus::BusEvent::Params {
                                    complete,
                                    received,
                                    expected,
                                    entries,
                                    ..
                                } => {
                                    if let Ok(mut s) = state.lock() {
                                        s.params = entries;
                                        s.params_complete = complete;
                                        s.params_received = received;
                                        s.params_expected = expected;
                                    }
                                }
                                groundctrl_core::proto::bus::BusEvent::Alarm { alarm, .. } => {
                                    if let Ok(mut s) = state.lock() {
                                        let now = SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .unwrap()
                                            .as_secs();
                                        s.alarms.push((now, alarm));
                                        if s.alarms.len() > 50 {
                                            s.alarms.remove(0);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        // 周期性刷新日志帧数
        {
            let hub = hub.clone();
            let state = state.clone();
            rt.spawn(async move {
                let mut ticker = tokio::time::interval(Duration::from_millis(500));
                loop {
                    ticker.tick().await;
                    let n = hub.log_frames().await;
                    if let Ok(mut s) = state.lock() {
                        s.log_frames = n;
                    }
                }
            });
        }

        // 默认以 SimLink 启动，打开即有遥测数据
        {
            let hub = hub.clone();
            rt.spawn(async move {
                hub.connect(link::share(link::sim::SimLink::new())).await;
            });
        }

        Self { state, hub, rt }
    }

    /// 连接指定类型的链路（先断开旧链路，再设为当前活动链路）
    fn connect(&self, kind: ConnectKind) {
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

    /// 断开当前活动链路
    fn disconnect(&self) {
        let hub = self.hub.clone();
        let state = self.state.clone();
        self.rt.spawn(async move {
            hub.disconnect().await;
            if let Ok(mut s) = state.lock() {
                s.log.push("链路已断开".into());
            }
        });
    }

    /// 请求参数（飞控 sys=1, comp=1）
    fn request_params(&self) {
        let hub = self.hub.clone();
        self.rt.spawn(async move {
            if let Err(e) = hub.request_params(1, 1).await {
                tracing::warn!("request params failed: {e}");
            }
        });
    }

    /// 写回一个参数
    fn set_param(&self, name: String, value: f32) {
        let hub = self.hub.clone();
        self.rt.spawn(async move {
            let _ = hub.set_param(1, 1, &name, value).await;
        });
    }

    /// 上传本地航点（MISSION_COUNT + 逐条 ITEM）
    fn upload_mission(&self) {
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

enum ConnectKind {
    Sim,
    Udp(String, String),
    Serial(String, u32),
}

impl eframe::App for GroundControlApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut state = self.state.lock().unwrap();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Ground Control");
                ui.label(format!("link: {}", state.link_status));
            });
            // 告警状态条
            alarm_bar(ui, &state.alarms);
        });

        egui::SidePanel::left("config").show(ctx, |ui| {
            ui.heading("连接");
            ui.label(format!("状态: {}", state.link_status));
            if ui.button("断开当前链路").clicked() {
                self.disconnect();
            }
            ui.separator();

            ui.collapsing("串口", |ui| {
                ui.text_edit_singleline(&mut state.serial_port);
                ui.add(egui::Slider::new(&mut state.baud, 9600..=921600).logarithmic(true));
                if ui.button("连接串口").clicked() {
                    self.connect(ConnectKind::Serial(
                        state.serial_port.clone(),
                        state.baud,
                    ));
                }
            });

            ui.collapsing("UDP", |ui| {
                ui.label("bind:");
                ui.text_edit_singleline(&mut state.udp_bind);
                ui.label("target:");
                ui.text_edit_singleline(&mut state.udp_target);
                if ui.button("连接 UDP").clicked() {
                    self.connect(ConnectKind::Udp(
                        state.udp_bind.clone(),
                        state.udp_target.clone(),
                    ));
                }
            });

            ui.separator();
            if ui.button("模拟链路 (Sim)").clicked() {
                self.connect(ConnectKind::Sim);
            }
        });

        // 中央区：Tab 切换
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (tk, label) in [
                    (TabKind::Telemetry, "遥测"),
                    (TabKind::Params, "参数"),
                    (TabKind::Mission, "航点"),
                    (TabKind::Map, "地图"),
                    (TabKind::Log, "日志"),
                ] {
                    if ui.selectable_label(state.tab == tk, label).clicked() {
                        state.tab = tk;
                        if tk == TabKind::Params {
                            self.request_params();
                        }
                    }
                }
            });
            ui.separator();
            match state.tab {
                TabKind::Telemetry => telemetry_panel(ui, &state.vehicle),
                TabKind::Params => params_panel(ui, &mut state, self),
                TabKind::Mission => mission_panel(ui, &mut state, self),
                TabKind::Map => map_panel(ui, &state),
                TabKind::Log => log_panel(ui, &state, &self.hub),
            }
        });

        egui::TopBottomPanel::bottom("log").show(ctx, |ui| {
            ui.label("日志");
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .show(ui, |ui| {
                    for line in state.log.iter().rev().take(20) {
                        ui.label(line);
                    }
                });
        });

        // 请求下一帧，保持刷新
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

fn telemetry_panel(ui: &mut egui::Ui, v: &VehicleModel) {
    ui.heading("遥测");
    ui.horizontal(|ui| {
        ui.label("在线:");
        ui.label(if v.online { "YES" } else { "NO" });
        ui.label(format!("sys={} comp={}", v.sys_id, v.comp_id));
    });

    ui.separator();
    ui.label("姿态 (deg)");
    ui.label(format!(
        "roll: {:.1}  pitch: {:.1}  yaw: {:.1}",
        v.attitude.roll.to_degrees(),
        v.attitude.pitch.to_degrees(),
        v.attitude.yaw.to_degrees()
    ));

    // 简单姿态仪（人工地平线）
    let p = ui.cursor();
    let size = 120.0;
    let (resp, painter) = ui.allocate_painter(
        egui::Vec2::new(size, size),
        egui::Sense::hover(),
    );
    let center = resp.rect.center();
    let r = size / 2.0 - 4.0;
    painter.circle_stroke(center, r, egui::Stroke::new(1.0_f32, egui::Color32::GRAY));
    // roll 旋转横线
    let roll = v.attitude.roll;
    let dx = (roll.cos() * r) as f32;
    let dy = (roll.sin() * r) as f32;
    painter.line_segment(
        [center - egui::Vec2::new(dx, dy), center + egui::Vec2::new(dx, dy)],
        egui::Stroke::new(2.0_f32, egui::Color32::GREEN),
    );
    let _ = p;

    ui.separator();
    ui.label("GPS");
    ui.label(format!(
        "lat: {:.6}  lon: {:.6}",
        v.gps.lat, v.gps.lon
    ));
    ui.label(format!(
        "alt: {:.1} m  rel: {:.1} m  hdg: {:.0}°",
        v.gps.alt, v.gps.relative_alt, v.gps.heading
    ));
    ui.label(format!("fix: {}  sats: {}", v.gps.fix_type, v.gps.satellites));

    ui.separator();
    ui.label("电池");
    ui.label(format!(
        "V: {:.2}  I: {:.2} A  rem: {}",
        v.battery.voltage,
        v.battery.current,
        v.battery
            .remaining_pct
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "未知".to_string())
    ));

    ui.separator();
    if let Some(hb) = &v.heartbeat {
        ui.label(format!(
            "HB: type={} ap={} status={} base_mode=0x{:02X}",
            hb.mav_type, hb.autopilot, hb.system_status, hb.base_mode
        ));
    }
}

/// 顶部告警状态条：根据当前最高等级告警着色
fn alarm_bar(ui: &mut egui::Ui, alarms: &[(u64, Alarm)]) {
    // 取最近一条仍处于激活态的告警（这里简化为显示最新一条）
    if let Some((_, last)) = alarms.last() {
        let (text, color) = match last.level {
            AlarmLevel::Critical => ("⚠ 严重告警", egui::Color32::RED),
            AlarmLevel::Warn => ("⚠ 警告", egui::Color32::YELLOW),
            AlarmLevel::Info => ("ℹ 提示", egui::Color32::BLUE),
        };
        ui.colored_label(color, format!("{text}: {} — {}", last.code, last.message));
    } else {
        ui.label("状态正常");
    }
}

/// 参数面板：显示参数缓存表，支持写回
fn params_panel(ui: &mut egui::Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("参数");
    ui.horizontal(|ui| {
        ui.label(format!(
            "进度: {}/{} {}",
            state.params_received,
            state.params_expected,
            if state.params_complete { "(完成)" } else { "(拉取中)" }
        ));
        if ui.button("重新请求").clicked() {
            app.request_params();
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for p in &state.params {
            ui.horizontal(|ui| {
                ui.label(&p.name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.4}", p.value));
                });
            });
        }
    });
}

/// 航点面板：本地航点列表 + 新增 + 上传
fn mission_panel(ui: &mut egui::Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("航点规划");
    ui.horizontal(|ui| {
        ui.label("lat:");
        ui.add(egui::DragValue::new(&mut state.wp_lat).speed(0.0001));
        ui.label("lon:");
        ui.add(egui::DragValue::new(&mut state.wp_lon).speed(0.0001));
        ui.label("alt:");
        ui.add(egui::DragValue::new(&mut state.wp_alt).speed(1.0));
        if ui.button("添加航点").clicked() {
            state
                .mission
                .push(Waypoint::nav(state.wp_lat, state.wp_lon, state.wp_alt));
        }
    });
    ui.horizontal(|ui| {
        if ui.button("上传到飞控").clicked() {
            app.upload_mission();
        }
        if ui.button("清空").clicked() {
            state.mission.clear();
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut to_remove: Option<usize> = None;
        for (i, wp) in state.mission.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", i));
                ui.label(format!(
                    "lat={:.6} lon={:.6} alt={:.1}m",
                    wp.lat, wp.lon, wp.alt
                ));
                if ui.button("删除").clicked() {
                    to_remove = Some(i);
                }
            });
        }
        if let Some(i) = to_remove {
            state.mission.remove(i);
        }
    });
}

/// 地图 HUD：简化经纬度散点 + 航迹线（离线，无瓦片）
fn map_panel(ui: &mut egui::Ui, state: &UiState) {
    ui.heading("地图 (简化 HUD)");
    ui.label("离线模式：以当前位置为中心绘制航迹（无地图瓦片）");

    let (resp, painter) = ui.allocate_painter(
        ui.available_size(),
        egui::Sense::hover(),
    );
    let rect = resp.rect;
    if state.trail.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "无 GPS 轨迹",
            egui::FontId::default(),
            egui::Color32::GRAY,
        );
        return;
    }

    // 计算经纬度的视觉范围（留边距）
    let lats: Vec<f64> = state.trail.iter().map(|(la, _)| *la).collect();
    let lons: Vec<f64> = state.trail.iter().map(|(_, lo)| *lo).collect();
    let min_lat = lats.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_lat = lats.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_lon = lons.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_lon = lons.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let lat_span = (max_lat - min_lat).max(1e-6);
    let lon_span = (max_lon - min_lon).max(1e-6);

    let pad: f64 = 16.0;
    let w: f64 = (rect.width() - 2.0 * pad as f32) as f64;
    let h: f64 = (rect.height() - 2.0 * pad as f32) as f64;

    let to_xy = |la: f64, lo: f64| -> egui::Pos2 {
        // 经度为 X，纬度为 Y（Y 轴翻转，北在上）
        let x = pad + ((lo - min_lon) / lon_span) * w;
        let y = pad + ((max_lat - la) / lat_span) * h;
        egui::Pos2::new(rect.min.x + x as f32, rect.min.y + y as f32)
    };

    // 航迹线
    let pts: Vec<egui::Pos2> = state.trail.iter().map(|(la, lo)| to_xy(*la, *lo)).collect();
    for w in pts.windows(2) {
        painter.line_segment(
            [w[0], w[1]],
            egui::Stroke::new(1.5_f32, egui::Color32::LIGHT_BLUE),
        );
    }
    // 当前点
    if let Some(&cur) = pts.last() {
        painter.circle_filled(cur, 4.0, egui::Color32::GREEN);
    }
}

/// 日志面板：显示帧数 + 简化回放状态
fn log_panel(ui: &mut egui::Ui, state: &UiState, hub: &Arc<TelemetryHub>) {
    ui.heading("飞行日志 (tlog)");
    ui.label(format!("已记录帧数: {}", state.log_frames));
    ui.label("日志在 TelemetryHub 中实时记录每条遥测帧（tlog 格式）。");
    ui.label("回放功能：通过 core 的 LogManager::from_tlog 解码并驱动总线。");
    let _ = hub; // 预留：未来导出按钮
}

fn main() -> eframe::Result<()> {
    // 初始化日志
    let _ = tracing_subscriber::fmt::try_init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 680.0])
            .with_title("Ground Control"),
        ..Default::default()
    };
    eframe::run_native(
        "Ground Control",
        options,
        Box::new(|_cc| Box::new(GroundControlApp::new())),
    )
}
