//! 脚本任务面板（P3-4）：编辑器 + 运行/停止 + 运行日志
//!
//! 脚本语法见 `groundctrl_core::services::script`（WAIT / SET_MODE / ARM /
//! DISARM / TAKEOFF / LAND / RTL / GOTO_WP / SET_PARAM / LOG / IF ... END）。

use egui::{Color32, RichText, Ui};

use crate::app::GroundControlApp;
use crate::state::UiState;

/// 示例脚本（首次使用时载入）
pub const SAMPLE_SCRIPT: &str = "\
# 示例任务：起飞 → 巡航 → 返航
LOG 任务开始
SET_MODE GUIDED
ARM
WAIT 2
TAKEOFF 10
WAIT 5
IF BATTERY_LT 30
  LOG 电量过低，提前返航
  RTL
END
WAIT 3
LAND
DISARM
LOG 任务完成
";

pub fn script_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    let lang = state.lang;
    ui.heading(lang.tr("脚本任务"));

    // 控制行：名称 + 载入示例 + 运行/停止
    ui.horizontal(|ui| {
        ui.label(lang.tr("任务名称:"));
        ui.add(egui::TextEdit::singleline(&mut state.script_name).desired_width(160.0));
        if ui.button(lang.tr("载入示例")).clicked() {
            state.script_text = SAMPLE_SCRIPT.to_string();
        }
        ui.separator();
        let run = ui.add_enabled(!state.script_running, egui::Button::new(lang.tr("运行")));
        if run.clicked() {
            app.run_script();
        }
        let stop = ui.add_enabled(state.script_running, egui::Button::new(lang.tr("停止")));
        if stop.clicked() {
            app.stop_script();
        }
    });

    if !state.script_msg.is_empty() {
        ui.label(RichText::new(&state.script_msg).color(Color32::YELLOW));
    }

    ui.separator();
    ui.label(RichText::new(lang.tr("脚本内容:")).strong());
    egui::ScrollArea::vertical()
        .max_height(240.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut state.script_text)
                    .code_editor()
                    .desired_rows(12)
                    .desired_width(f32::INFINITY),
            );
        });

    ui.separator();
    ui.label(RichText::new(lang.tr("运行日志")).strong());
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(ui, |ui| {
            for line in state.script_log.iter().rev().take(200) {
                ui.monospace(line);
            }
        });
}
