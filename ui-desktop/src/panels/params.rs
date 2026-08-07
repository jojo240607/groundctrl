//! 参数面板：显示参数缓存表，支持选中后写回飞控。

use egui::{DragValue, Id, Ui};

use crate::app::GroundControlApp;
use crate::state::UiState;

pub fn params_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
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
    let selected: Option<String> = ui.data(|d| d.get_temp(Id::new("param_selected")));
    let mut edit_val: f32 = ui
        .data(|d| d.get_temp(Id::new("param_edit_val")))
        .unwrap_or(0.0);

    egui::ScrollArea::vertical().show(ui, |ui| {
        for p in &state.params {
            let is_sel = selected.as_deref() == Some(p.name.as_str());
            ui.horizontal(|ui| {
                if ui.selectable_label(is_sel, &p.name).clicked() {
                    ui.data_mut(|d| d.insert_temp(Id::new("param_selected"), p.name.clone()));
                    ui.data_mut(|d| d.insert_temp(Id::new("param_edit_val"), p.value));
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
            ui.add(DragValue::new(&mut edit_val).speed(0.01));
            ui.data_mut(|d| d.insert_temp(Id::new("param_edit_val"), edit_val));
            if ui.button("发送到飞控").clicked() {
                app.set_param(name.clone(), edit_val);
            }
        });
    } else {
        ui.label("点击左侧参数名可选中并修改后写入飞控");
    }
}
