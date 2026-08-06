//! 飞控地面站 — 桌面端（egui / eframe）
//!
//! 启动 tokio runtime 驱动 core 的 TelemetryHub，egui 订阅遥测快照并显示。

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use groundctrl_core::link::{self, LinkHandle};
use groundctrl_core::services::TelemetryHub;
use groundctrl_core::vehicle::VehicleModel;

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
                                s.vehicle = vm;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }

        // 订阅总线事件（链路状态）
        {
            let hub = hub.clone();
            let state = state.clone();
            rt.spawn(async move {
                let mut rx = hub.bus().subscribe();
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            if let groundctrl_core::proto::bus::BusEvent::LinkState { link, open } =
                                ev
                            {
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
                        }
                        Err(_) => break,
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
            ui.heading("Ground Control — MAVLink GCS");
            ui.label(format!("link: {}", state.link_status));
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

        egui::CentralPanel::default().show(ctx, |ui| {
            telemetry_panel(ui, &state.vehicle);
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
        "V: {:.2}  I: {:.2} A  rem: {}%",
        v.battery.voltage, v.battery.current, v.battery.remaining_pct
    ));

    ui.separator();
    if let Some(hb) = &v.heartbeat {
        ui.label(format!(
            "HB: type={} ap={} status={} base_mode=0x{:02X}",
            hb.mav_type, hb.autopilot, hb.system_status, hb.base_mode
        ));
    }
}

fn main() -> eframe::Result<()> {
    // 初始化日志
    let _ = tracing_subscriber::fmt::try_init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 600.0])
            .with_title("Ground Control"),
        ..Default::default()
    };
    eframe::run_native(
        "Ground Control",
        options,
        Box::new(|_cc| Box::new(GroundControlApp::new())),
    )
}
