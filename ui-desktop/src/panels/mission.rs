//! 航点面板：本地航点列表 + 新增/删除/排序 + 上传 / 清空。

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

    // 点击地图添加航点时使用的默认高度，可在此调整
    ui.horizontal(|ui| {
        if ui.button("上传到飞控").clicked() {
            app.upload_mission();
        }
        if ui.button("清空").clicked() {
            state.mission.clear();
        }
        ui.label(format!("共 {} 个航点", state.mission.len()));
        if ui.button("在地图上点选添加").clicked() {
            state.map_click_add_wp = !state.map_click_add_wp;
        }
    });

    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut to_remove: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;

        for i in 0..state.mission.len() {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", i));
                // 可编辑高度
                ui.add(DragValue::new(&mut state.mission[i].alt).speed(0.5).prefix("alt=").suffix("m"));
                let wp = &state.mission[i];
                ui.label(format!("lat={:.6} lon={:.6}", wp.lat, wp.lon));
                if ui.button("↑").clicked() {
                    if i > 0 {
                        move_up = Some(i);
                    }
                }
                if ui.button("↓").clicked() {
                    if i + 1 < state.mission.len() {
                        move_down = Some(i);
                    }
                }
                if ui.button("删除").clicked() {
                    to_remove = Some(i);
                }
            });
        }

        if let Some(i) = to_remove {
            state.mission.remove(i);
        }
        if let Some(i) = move_up {
            state.mission.swap(i, i - 1);
        }
        if let Some(i) = move_down {
            state.mission.swap(i, i + 1);
        }

        if state.mission.is_empty() {
            ui.label("尚无航点。在上方输入坐标添加，或勾选「在地图上点选添加」后点击地图。");
        }
    });
}
