//! 飞控地面站 — 桌面端（egui / eframe）
//!
//! 启动 tokio runtime 驱动 core 的 TelemetryHub，egui 订阅遥测快照并显示。

mod app;
mod panels;
mod state;
mod widgets;

use eframe::egui;

use app::{GroundControlApp, TabKind};
use widgets::alarm_bar::alarm_bar;

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
            panels::connection::connection_panel(ui, &mut state, self);
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
                TabKind::Telemetry => panels::telemetry::telemetry_panel(ui, &state.vehicle),
                TabKind::Params => panels::params::params_panel(ui, &mut state, self),
                TabKind::Mission => panels::mission::mission_panel(ui, &mut state, self),
                TabKind::Map => panels::map::map_panel(ui, &mut state, self),
                TabKind::Log => panels::log::log_panel(ui, &mut state, self),
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
        ctx.request_repaint_after(app::REPAINT_INTERVAL);
    }
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
