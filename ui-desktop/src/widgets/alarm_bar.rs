//! 顶部告警状态条：根据当前最高等级告警着色，并显示机队在线概览。

use egui::{Color32, Ui};

use groundctrl_core::services::alarms::AlarmLevel;

use crate::state::UiState;

/// 渲染顶部告警状态条；取最近一条告警，按等级着色。
pub fn alarm_bar(ui: &mut Ui, state: &UiState) {
    // 机队在线概览
    let online = state.vehicles.len();
    ui.horizontal(|ui| {
        let fleet_txt = if online == 0 {
            "机队: 无飞机在网".to_string()
        } else {
            let sel = state
                .selected_sys
                .map(|s| format!(" (查看 SYS {s})"))
                .unwrap_or_default();
            format!("机队: {} 架在网{}", online, sel)
        };
        ui.label(egui::RichText::new(fleet_txt).weak());

        ui.separator();

        if let Some((_, last)) = state.alarms.last() {
            let (text, color) = match last.level {
                AlarmLevel::Critical => ("⚠ 严重告警", Color32::RED),
                AlarmLevel::Warn => ("⚠ 警告", Color32::YELLOW),
                AlarmLevel::Info => ("ℹ 提示", Color32::BLUE),
            };
            ui.colored_label(color, format!("{text}: {} — {}", last.code, last.message));
        } else {
            ui.label("状态正常");
        }
    });
}
