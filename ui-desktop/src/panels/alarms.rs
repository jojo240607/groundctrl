//! 告警面板：查看历史告警列表，按等级筛选、清除。

use egui::{Color32, RichText, Ui};

use groundctrl_core::services::alarms::{AlarmLevel, Alarm};

use crate::app::GroundControlApp;
use crate::state::UiState;

pub fn alarms_panel(ui: &mut Ui, state: &mut UiState, _app: &GroundControlApp) {
    ui.heading("告警");

    ui.horizontal(|ui| {
        let counts = summarize(&state.alarms);
        ui.label(RichText::new(format!("严重 {}", counts.0)).color(Color32::RED));
        ui.label(RichText::new(format!("警告 {}", counts.1)).color(Color32::YELLOW));
        ui.label(RichText::new(format!("提示 {}", counts.2)).color(Color32::BLUE));
        ui.separator();
        ui.label(format!("共 {} 条", state.alarms.len()));
        if ui.button("清除全部").clicked() {
            state.alarms.clear();
        }
    });

    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (ts, a) in state.alarms.iter().rev() {
            let col = level_color(a.level);
            ui.horizontal(|ui| {
                ui.label(RichText::new(fmt_time(*ts)).weak());
                ui.label(RichText::new(level_tag(a.level)).color(col).strong());
                ui.label(RichText::new(a.code).color(col));
                ui.label(&a.message);
            });
        }
        if state.alarms.is_empty() {
            ui.label("无告警记录");
        }
    });
}

fn summarize(alarms: &[(u64, Alarm)]) -> (usize, usize, usize) {
    let mut c = (0, 0, 0);
    for (_, a) in alarms {
        match a.level {
            AlarmLevel::Critical => c.0 += 1,
            AlarmLevel::Warn => c.1 += 1,
            AlarmLevel::Info => c.2 += 1,
        }
    }
    c
}

fn level_color(l: AlarmLevel) -> Color32 {
    match l {
        AlarmLevel::Critical => Color32::RED,
        AlarmLevel::Warn => Color32::YELLOW,
        AlarmLevel::Info => Color32::BLUE,
    }
}

fn level_tag(l: AlarmLevel) -> &'static str {
    match l {
        AlarmLevel::Critical => "[严重]",
        AlarmLevel::Warn => "[警告]",
        AlarmLevel::Info => "[提示]",
    }
}

fn fmt_time(ts: u64) -> String {
    if ts == 0 {
        return "--".into();
    }
    // 显示为本地时间 HH:MM:SS（基于 Unix 秒）
    let secs = ts % 86400;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}
