//! 航点面板：本地航点列表 + 新增 + 上传 / 清空。

use egui::{DragValue, Ui};

use groundctrl_core::vehicle::mission::Waypoint;

use crate::app::GroundControlApp;
use crate::state::UiState;

pub fn mission_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("航点规划");
    ui.horizontal(|ui| {
        ui.label("lat:");
        ui.add(DragValue::new(&mut state.wp_lat).speed(0.0001));
        ui.label("lon:");
        ui.add(DragValue::new(&mut state.wp_lon).speed(0.0001));
        ui.label("alt:");
        ui.add(DragValue::new(&mut state.wp_alt).speed(1.0));
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
