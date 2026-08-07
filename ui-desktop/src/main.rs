//! 飞控地面站 — 桌面端（egui / eframe）
//!
//! 启动 tokio runtime 驱动 core 的 TelemetryHub，egui 订阅遥测快照并显示。

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use groundctrl_core::link::{self, LinkHandle};
use groundctrl_core::services::alarms::{Alarm, AlarmLevel};
use groundctrl_core::services::log::LogManager;
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
    /// 地图缩放级别（越大越近）
    map_zoom: f64,
    /// 导出 / 导入反馈信息
    export_msg: String,
    import_msg: String,
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
                TabKind::Map => map_panel(ui, &mut state),
                TabKind::Log => log_panel(ui, &mut state, self),
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

    // 人工地平仪（artificial horizon）
    let size = 160.0;
    let (resp, mut painter) = ui.allocate_painter(
        egui::Vec2::new(size, size),
        egui::Sense::hover(),
    );
    let center = resp.rect.center();
    let r = size / 2.0 - 4.0;

    // 用裁剪区把地平仪限制在圆形内
    let mut pitch = v.attitude.pitch; // 弧度
    let roll = v.attitude.roll; // 弧度
    // 限制极端角度，避免几何发散
    pitch = pitch.clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);

    // 俯仰在屏幕上偏移：每度约 2px
    let pitch_px = (pitch.to_degrees() * 2.0) as f32;
    // roll 旋转
    let cs = roll.cos() as f32;
    let sn = roll.sin() as f32;

    // 地平线在旋转坐标系下的 y 偏移（未旋转前）
    let horizon_y = pitch_px; // 正俯仰（抬头）=> 地平线下移
    // 旋转后的方向向量：屏幕 x 轴在机体坐标系下
    let rot = |x: f32, y: f32| -> egui::Pos2 {
        egui::Pos2::new(center.x + (x * cs - y * sn), center.y + (x * sn + y * cs))
    };

    // 裁剪到圆
    let clip_rect = egui::Rect::from_center_size(center, egui::Vec2::splat(size));
    painter.set_clip_rect(clip_rect);

    // 天空 / 地面填充（以地平线为界，整块涂色后旋转）
    let far = r + 60.0;
    // 天空多边形（地平线以上）
    let sky = vec![
        rot(-far, -far + horizon_y),
        rot(far, -far + horizon_y),
        rot(far, -far),
        rot(-far, -far),
    ];
    painter.add(egui::Shape::convex_polygon(
        sky,
        egui::Color32::from_rgb(70, 130, 200),
        egui::Stroke::NONE,
    ));
    // 地面多边形（地平线以下）
    let ground = vec![
        rot(-far, far + horizon_y),
        rot(far, far + horizon_y),
        rot(far, far),
        rot(-far, far),
    ];
    painter.add(egui::Shape::convex_polygon(
        ground,
        egui::Color32::from_rgb(120, 90, 50),
        egui::Stroke::NONE,
    ));

    // 地平线（白线）
    let hl_a = rot(-r, horizon_y);
    let hl_b = rot(r, horizon_y);
    painter.line_segment([hl_a, hl_b], egui::Stroke::new(2.0_f32, egui::Color32::WHITE));

    // 俯仰刻度（每 10° 一条，带标号）
    for deg in [-30, -20, -10, 10, 20, 30] {
        let y = horizon_y - (deg as f32) * 2.0; // 注意符号：抬头时该刻度在地平线上方（机体坐标 -y）
        let len = if deg % 20 == 0 { 24.0 } else { 14.0 };
        let a = rot(-len, y);
        let b = rot(len, y);
        painter.line_segment([a, b], egui::Stroke::new(1.0_f32, egui::Color32::WHITE));
        if deg % 20 == 0 {
            let tx = rot(len + 6.0, y);
            painter.text(
                tx,
                egui::Align2::LEFT_CENTER,
                format!("{deg}"),
                egui::FontId::proportional(10.0),
                egui::Color32::WHITE,
            );
        }
    }

    // 恢复裁剪（球体描边在裁剪外画）
    painter.set_clip_rect(egui::Rect::EVERYTHING);

    // 外圆 + 固定机体符号（不随姿态旋转）
    painter.circle_stroke(center, r, egui::Stroke::new(1.5_f32, egui::Color32::GRAY));
    // 机体参考符号：中心横杠 + 小翼
    painter.line_segment(
        [center - egui::Vec2::new(20.0, 0.0), center - egui::Vec2::new(6.0, 0.0)],
        egui::Stroke::new(2.5_f32, egui::Color32::YELLOW),
    );
    painter.line_segment(
        [center + egui::Vec2::new(6.0, 0.0), center + egui::Vec2::new(20.0, 0.0)],
        egui::Stroke::new(2.5_f32, egui::Color32::YELLOW),
    );
    painter.line_segment(
        [center, center - egui::Vec2::new(0.0, 8.0)],
        egui::Stroke::new(2.5_f32, egui::Color32::YELLOW),
    );

    ui.label(format!(
        "roll: {:.0}°  pitch: {:.0}°  yaw: {:.0}°",
        roll.to_degrees(),
        pitch.to_degrees(),
        v.attitude.yaw.to_degrees()
    ));

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

    // 选中参数写回：用 egui 临时存储记录选中项与编辑值
    let selected: Option<String> = ui.data(|d| d.get_temp(egui::Id::new("param_selected")));
    let mut edit_val: f32 = ui
        .data(|d| d.get_temp(egui::Id::new("param_edit_val")))
        .unwrap_or(0.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        for p in &state.params {
            let is_sel = selected.as_deref() == Some(p.name.as_str());
            ui.horizontal(|ui| {
                if ui.selectable_label(is_sel, &p.name).clicked() {
                    ui.data_mut(|d| d.insert_temp(egui::Id::new("param_selected"), p.name.clone()));
                    ui.data_mut(|d| d.insert_temp(egui::Id::new("param_edit_val"), p.value));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.4}", p.value));
                });
            });
        }
    });

    ui.separator();
    if let Some(name) = &selected {
        ui.horizontal(|ui| {
            ui.label(format!("写入 [{name}]:"));
            ui.add(egui::DragValue::new(&mut edit_val).speed(0.01));
            ui.data_mut(|d| d.insert_temp(egui::Id::new("param_edit_val"), edit_val));
            if ui.button("发送到飞控").clicked() {
                app.set_param(name.clone(), edit_val);
            }
        });
    } else {
        ui.label("点击左侧参数名可选中并修改后写入飞控");
    }
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

/// 地图 HUD：简化经纬度散点 + 航迹线 + 航点叠加 + 指北针（离线，无瓦片）
fn map_panel(ui: &mut egui::Ui, state: &mut UiState) {
    ui.heading("地图 (简化 HUD)");
    ui.label("离线模式：以当前位置为中心绘制航迹与航点（无地图瓦片）");
    ui.horizontal(|ui| {
        ui.label("缩放:");
        ui.add(egui::Slider::new(&mut state.map_zoom, 2.0..=18.0).logarithmic(true));
        ui.label(format!("z={:.1}", state.map_zoom));
    });

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

    // 以当前位置为视图中心；zoom 越大视野越窄
    let cur = *state.trail.last().unwrap();
    // 视野半宽（度）：zoom 18 => 约 0.002°，zoom 2 => 约 1.0°
    let half_span = (1.0 / (state.map_zoom * state.map_zoom)) + 0.0008;
    let min_lat = cur.0 - half_span;
    let max_lat = cur.0 + half_span;
    let min_lon = cur.1 - half_span;
    let max_lon = cur.1 + half_span;
    let lat_span = (max_lat - min_lat).max(1e-9);
    let lon_span = (max_lon - min_lon).max(1e-9);

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

    // 本地航点叠加（橙色方块）
    for wp in &state.mission {
        let p = to_xy(wp.lat, wp.lon);
        painter.rect_filled(
            egui::Rect::from_center_size(p, egui::Vec2::splat(8.0)),
            1.0,
            egui::Color32::GOLD,
        );
    }

    // 当前点
    if let Some(&cur_p) = pts.last() {
        painter.circle_filled(cur_p, 4.0, egui::Color32::GREEN);
    }

    // 指北针（左上角小指示器）
    let n_size = 26.0;
    let n_center = rect.min + egui::Vec2::new(n_size + 10.0, n_size + 10.0);
    painter.circle_stroke(n_center, n_size, egui::Stroke::new(1.0_f32, egui::Color32::GRAY));
    painter.line_segment(
        [n_center, n_center - egui::Vec2::new(0.0, n_size)],
        egui::Stroke::new(2.0_f32, egui::Color32::RED),
    );
    painter.text(
        n_center - egui::Vec2::new(0.0, n_size + 9.0),
        egui::Align2::CENTER_CENTER,
        "N",
        egui::FontId::proportional(12.0),
        egui::Color32::RED,
    );
}

/// 日志面板：显示帧数 + tlog 保存/加载 + CSV/KML 导出
fn log_panel(ui: &mut egui::Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("飞行日志 (tlog)");
    ui.label(format!("已记录帧数: {}", state.log_frames));
    ui.label("日志在 TelemetryHub 中实时记录每条遥测帧（tlog 格式）。");

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("保存 tlog...").clicked() {
            let hub = app.hub.clone();
            let rt = app.rt.handle().clone();
            let st = app.state.clone();
            rt.spawn(async move {
                if let Some(path) = rfd::AsyncFileDialog::new()
                    .set_title("保存飞行日志")
                    .set_file_name("flight.tlog")
                    .save_file()
                    .await
                {
                    let path = path.path().to_path_buf();
                    let log_arc = hub.log();
                    let res = {
                        let log = log_arc.lock().await;
                        log.save_file(path.to_str().unwrap_or("flight.tlog"))
                    };
                    if let Ok(mut s) = st.lock() {
                        s.export_msg = match res {
                            Ok(_) => format!("已保存: {}", path.display()),
                            Err(e) => format!("保存失败: {e}"),
                        };
                    }
                }
            });
        }
        if ui.button("加载 tlog...").clicked() {
            let hub = app.hub.clone();
            let rt = app.rt.handle().clone();
            let st = app.state.clone();
            rt.spawn(async move {
                if let Some(path) = rfd::AsyncFileDialog::new()
                    .set_title("加载飞行日志")
                    .add_filter("tlog", &["tlog"])
                    .pick_file()
                    .await
                {
                    let path = path.path().to_path_buf();
                    let sp = path.to_str().unwrap_or("flight.tlog").to_string();
                    let loaded = LogManager::load_file(&sp);
                    let mut msg = String::new();
                    if let Ok(lm) = &loaded {
                        // 用载入的日志替换当前 hub 日志
                        let log_arc = hub.log();
                        *log_arc.lock().await = lm.clone();
                        msg = format!("已加载 {} 帧: {}", lm.len(), path.display());
                    } else if let Err(e) = &loaded {
                        msg = format!("加载失败: {e}");
                    }
                    if let Ok(mut s) = st.lock() {
                        s.import_msg = msg;
                    }
                }
            });
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("导出轨迹 CSV...").clicked() {
            let rt = app.rt.handle().clone();
            let st = app.state.clone();
            rt.spawn(async move {
                if let Some(path) = rfd::AsyncFileDialog::new()
                    .set_title("导出轨迹 CSV")
                    .set_file_name("track.csv")
                    .save_file()
                    .await
                {
                    let path = path.path().to_path_buf();
                    let (csv, n) = {
                        let s = st.lock().unwrap();
                        (export_track_csv(&s.trail), s.trail.len())
                    };
                    let res = std::fs::write(&path, csv);
                    if let Ok(mut s) = st.lock() {
                        s.export_msg = match res {
                            Ok(_) => format!("已导出 CSV: {n} 点 ({})", path.display()),
                            Err(e) => format!("导出失败: {e}"),
                        };
                    }
                }
            });
        }
        if ui.button("导出轨迹 KML...").clicked() {
            let rt = app.rt.handle().clone();
            let st = app.state.clone();
            rt.spawn(async move {
                if let Some(path) = rfd::AsyncFileDialog::new()
                    .set_title("导出轨迹 KML")
                    .set_file_name("track.kml")
                    .save_file()
                    .await
                {
                    let path = path.path().to_path_buf();
                    let (kml, n) = {
                        let s = st.lock().unwrap();
                        (export_track_kml(&s.trail, &s.mission), s.trail.len())
                    };
                    let res = std::fs::write(&path, kml);
                    if let Ok(mut s) = st.lock() {
                        s.export_msg = match res {
                            Ok(_) => format!("已导出 KML: {n} 点 ({})", path.display()),
                            Err(e) => format!("导出失败: {e}"),
                        };
                    }
                }
            });
        }
    });

    if !state.export_msg.is_empty() {
        ui.label(egui::RichText::new(&state.export_msg).color(egui::Color32::GREEN));
    }
    if !state.import_msg.is_empty() {
        ui.label(egui::RichText::new(&state.import_msg).color(egui::Color32::LIGHT_BLUE));
    }
    ui.label("CSV/KML 由当前 GPS 轨迹（与本地航点）生成，可用于 Google Earth / 表格分析。");
}

/// 将 GPS 轨迹导出为 CSV（seq,lat,lon）
fn export_track_csv(trail: &[(f64, f64)]) -> String {
    let mut s = String::from("seq,latitude,longitude\n");
    for (i, (la, lo)) in trail.iter().enumerate() {
        s.push_str(&format!("{i},{:.7},{:.7}\n", la, lo));
    }
    s
}

/// 将 GPS 轨迹与航点导出为 KML（LineString + Point Placemarks）
fn export_track_kml(trail: &[(f64, f64)], mission: &[Waypoint]) -> String {
    let mut coords: String = String::new();
    for (la, lo) in trail {
        coords.push_str(&format!("{:.7},{:.7},0\n", lo, la));
    }
    let mut wps: String = String::new();
    for (i, wp) in mission.iter().enumerate() {
        wps.push_str(&format!(
            "    <Placemark>\n      <name>WP#{i}</name>\n      <Point><coordinates>{:.7},{:.7},{}</coordinates></Point>\n    </Placemark>\n",
            wp.lon, wp.lat, wp.alt
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2">
  <Document>
    <name>GroundControl Track</name>
    <Placemark>
      <name>Track</name>
      <LineString>
        <coordinates>
{coords}        </coordinates>
      </LineString>
    </Placemark>
{wps}  </Document>
</kml>
"#
    )
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
