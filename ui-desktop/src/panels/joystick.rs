//! 摇杆 / 键盘操控面板（P2-3）：
//! - 键盘：W/S 俯仰、A/D 横滚、Q/E 偏航、R/F 油门
//! - 摇杆：右杆横滚/俯仰、左杆油门/偏航（gilrs）
//! 以 RC_CHANNELS_OVERRIDE 下发，飞控按 MAVLink 超时（约 3s）自动释放。

use egui::{Color32, RichText, Ui};

use crate::app::GroundControlApp;
use crate::state::UiState;

/// 轴值（-1..1）转 PWM（1000~2000，带死区）
fn axis_to_pwm(axis: f32) -> u16 {
    const DEADZONE: f32 = 0.05;
    let a = if axis.abs() < DEADZONE { 0.0 } else { axis };
    (1500.0 + a * 400.0).clamp(1000.0, 2000.0) as u16
}

pub fn joystick_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("摇杆 / 键盘操控");
    ui.label(
        RichText::new(
            "以 RC_CHANNELS_OVERRIDE 覆盖遥控通道（1 横滚 / 2 俯仰 / 3 油门 / 4 偏航）。\
             仅建议在 STABILIZE 等手动模式下使用；约 3s 无更新飞控自动释放覆盖。",
        )
        .color(Color32::GRAY),
    );

    ui.separator();

    // ---- 键盘操控 ----
    ui.horizontal(|ui| {
        ui.checkbox(&mut state.ctrl_kb_enabled, "启用键盘操控");
        ui.label(RichText::new("W/S 俯仰 · A/D 横滚 · Q/E 偏航 · R/F 油门").color(Color32::GRAY));
    });

    // ---- 摇杆操控 ----
    ui.horizontal(|ui| {
        ui.checkbox(&mut state.ctrl_js_enabled, "启用摇杆操控");
        if state.joystick_name.is_empty() {
            ui.label(RichText::new("未检测到手柄").color(Color32::GRAY));
        } else {
            ui.label(RichText::new(&state.joystick_name).color(Color32::LIGHT_BLUE));
            ui.label(RichText::new("右杆 横滚/俯仰 · 左杆 油门/偏航").color(Color32::GRAY));
        }
    });

    // ---- 通道预览（最近一次发送值）----
    ui.separator();
    ui.label(RichText::new("通道输出预览").strong());
    let names = ["1 横滚", "2 俯仰", "3 油门", "4 偏航"];
    for (i, n) in names.iter().enumerate() {
        let v = state.ctrl_last_chans[i];
        ui.horizontal(|ui| {
            ui.label(format!("{n}: {v}"));
            let (resp, painter) =
                ui.allocate_painter(egui::Vec2::new(200.0, 10.0), egui::Sense::hover());
            let frac = ((v as f32 - 1000.0) / 1000.0).clamp(0.0, 1.0);
            painter.rect_filled(resp.rect, 2.0, Color32::from_gray(40));
            painter.rect_filled(
                egui::Rect::from_min_size(
                    resp.rect.min,
                    egui::Vec2::new(resp.rect.width() * frac, resp.rect.height()),
                ),
                2.0,
                if (1000..=2000).contains(&v) {
                    Color32::GREEN
                } else {
                    Color32::DARK_GRAY
                },
            );
        });
    }

    // 清除覆盖按钮
    if ui.button("停止操控（清除覆盖）").clicked() {
        state.ctrl_kb_enabled = false;
        state.ctrl_js_enabled = false;
        state.ctrl_last_chans = [0; 8];
        let hub = app.hub.clone();
        let rt = app.rt.handle().clone();
        let (sys, comp) = target(state);
        rt.spawn(async move {
            let _ = hub.clear_rc_override(sys, comp).await;
        });
    }

    // ---- 节流发送（10Hz）----
    let now = ui.input(|i| i.time);
    if now - state.ctrl_last_send >= 0.1 {
        let kb = state.ctrl_kb_enabled;
        let js = state.ctrl_js_enabled;
        if kb || js {
            let mut chans = [0u16; 8];
            if kb {
                // 键盘：按住全幅（1900/1100），松开回中 1500
                let key = |k: egui::Key| ui.input(|i| i.key_down(k));
                chans[0] = if key(egui::Key::A) {
                    1100
                } else if key(egui::Key::D) {
                    1900
                } else {
                    1500
                };
                chans[1] = if key(egui::Key::W) {
                    1900
                } else if key(egui::Key::S) {
                    1100
                } else {
                    1500
                };
                chans[2] = if key(egui::Key::R) {
                    1900
                } else if key(egui::Key::F) {
                    1100
                } else {
                    1500
                };
                chans[3] = if key(egui::Key::Q) {
                    1100
                } else if key(egui::Key::E) {
                    1900
                } else {
                    1500
                };
            } else {
                // 摇杆：右杆 横滚(X)/俯仰(-Y)，左杆 油门(-Y)/偏航(X)
                let ax = state.joystick_axes;
                chans[0] = axis_to_pwm(ax[2]);
                chans[1] = axis_to_pwm(-ax[3]);
                chans[2] = axis_to_pwm(-ax[1]);
                chans[3] = axis_to_pwm(ax[0]);
            }
            state.ctrl_last_send = now;
            state.ctrl_last_chans = chans;
            let (sys, comp) = target(state);
            let hub = app.hub.clone();
            let rt = app.rt.handle().clone();
            rt.spawn(async move {
                let _ = hub.send_rc_override(sys, comp, chans).await;
            });
        }
    }
}

/// 目标机（当前选中飞机，未连接默认 1/1）
fn target(state: &UiState) -> (u8, u8) {
    if state.vehicle.online {
        (state.vehicle.sys_id, state.vehicle.comp_id)
    } else {
        (1, 1)
    }
}
