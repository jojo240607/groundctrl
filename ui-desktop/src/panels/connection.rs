//! 连接面板：侧栏连接配置（串口 / UDP / Sim）与断开按钮。

use egui::{Slider, Ui};

use crate::app::{ConnectKind, GroundControlApp};
use crate::state::UiState;

pub fn connection_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    let lang = state.lang;
    ui.heading(lang.tr("连接"));
    ui.label(format!("{} {}", lang.tr("状态:"), state.link_status));
    if ui.button(lang.tr("断开当前链路")).clicked() {
        app.disconnect();
    }
    ui.separator();

    ui.collapsing(lang.tr("串口"), |ui| {
        ui.text_edit_singleline(&mut state.serial_port);
        ui.add(Slider::new(&mut state.baud, 9600..=921600).logarithmic(true));
        if ui.button(lang.tr("连接串口")).clicked() {
            app.connect(ConnectKind::Serial(
                state.serial_port.clone(),
                state.baud,
            ));
            app.save_settings();
        }
    });

    ui.collapsing("UDP", |ui| {
        ui.label("bind:");
        ui.text_edit_singleline(&mut state.udp_bind);
        ui.label("target:");
        ui.text_edit_singleline(&mut state.udp_target);
        if ui.button(lang.tr("连接 UDP")).clicked() {
            app.connect(ConnectKind::Udp(
                state.udp_bind.clone(),
                state.udp_target.clone(),
            ));
            app.save_settings();
        }
    });

    ui.separator();
    if ui.button(lang.tr("模拟链路 (Sim)")).clicked() {
        app.connect(ConnectKind::Sim);
    }
}
