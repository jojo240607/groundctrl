//! 顶部告警状态条：根据当前最高等级告警着色。

use egui::{Color32, Ui};

use groundctrl_core::services::alarms::{Alarm, AlarmLevel};

/// 渲染顶部告警状态条；取最近一条告警，按等级着色。
pub fn alarm_bar(ui: &mut Ui, alarms: &[(u64, Alarm)]) {
    if let Some((_, last)) = alarms.last() {
        let (text, color) = match last.level {
            AlarmLevel::Critical => ("⚠ 严重告警", Color32::RED),
            AlarmLevel::Warn => ("⚠ 警告", Color32::YELLOW),
            AlarmLevel::Info => ("ℹ 提示", Color32::BLUE),
        };
        ui.colored_label(color, format!("{text}: {} — {}", last.code, last.message));
    } else {
        ui.label("状态正常");
    }
}
