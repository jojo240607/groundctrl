//! 遥测面板：姿态、人工地平仪、空速/高度仪表、GPS、电池、心跳、飞行控制、遥控通道。

use egui::{Color32, RichText, Ui};

use groundctrl_core::vehicle::VehicleModel;

use crate::app::{FlightCommand, GroundControlApp};
use crate::state::UiState;
use crate::widgets::horizon::attitude_indicator;
use crate::widgets::round_gauge::round_gauge;

/// ArduPilot Copter 模式表（custom_mode -> 名称）
const COPTER_MODES: &[(u32, &str)] = &[
    (0, "STABILIZE"),
    (2, "ALT_HOLD"),
    (3, "AUTO"),
    (4, "GUIDED"),
    (5, "LOITER"),
    (6, "RTL"),
    (9, "LAND"),
    (13, "SPORT"),
    (16, "POSHOLD"),
];

pub fn telemetry_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    // 克隆快照，避免持有借用时无法修改 state（如写日志）
    let v = state.vehicle.clone();
    let lang = state.lang;
    ui.heading(lang.tr("遥测"));

    // 顶部状态行：在线徽章 + 系统标识
    ui.horizontal(|ui| {
        let (txt, col) = if v.online {
            ("● 在线", Color32::GREEN)
        } else {
            ("○ 离线", Color32::GRAY)
        };
        ui.label(RichText::new(txt).color(col).strong());
        ui.separator();
        ui.label(format!("SYS {} · COMP {}", v.sys_id, v.comp_id));
        if let Some(hb) = &v.heartbeat {
            ui.separator();
            let armed = v.is_armed();
            ui.label(RichText::new(if armed { "ARMED" } else { "DISARMED" }).color(
                if armed { Color32::RED } else { Color32::YELLOW },
            ));
            ui.separator();
            let fm = v.flight_mode_name();
            ui.label(RichText::new(format!("{} {fm}", lang.tr("模式:"))).strong());
        }
    });

    ui.separator();
    flight_control_section(ui, state, app, &v);

    ui.separator();
    ui.label(RichText::new(lang.tr("姿态")).strong());
    ui.horizontal(|ui| {
        attitude_indicator(ui, &v, 160.0);
        ui.vertical(|ui| {
            kv(ui, "Roll", &format!("{:.1}°", v.attitude.roll.to_degrees()));
            kv(ui, "Pitch", &format!("{:.1}°", v.attitude.pitch.to_degrees()));
            kv(ui, "Yaw", &format!("{:.1}°", v.attitude.yaw.to_degrees()));
        });
    });

    ui.separator();
    ui.label(RichText::new("空速 / 高度").strong());
    ui.horizontal(|ui| {
        let (ra, pa) = ui.allocate_painter(egui::Vec2::new(150.0, 150.0), egui::Sense::hover());
        round_gauge(&pa, ra.rect.center(), 66.0, v.air.airspeed, "空速 m/s", 0.0, 40.0);
        let (rh, ph) = ui.allocate_painter(egui::Vec2::new(150.0, 150.0), egui::Sense::hover());
        round_gauge(
            &ph,
            rh.rect.center(),
            66.0,
            v.gps.relative_alt,
            "高度 m",
            0.0,
            120.0,
        );
    });
    ui.horizontal(|ui| {
        kv(ui, "地速", &format!("{:.1} m/s", v.air.groundspeed));
        kv(ui, "爬升", &format!("{:.1} m/s", v.air.climb));
        kv(ui, "油门", &format!("{}%", v.air.throttle));
    });

    ui.separator();
    rc_section(ui, &v, state.lang);

    ui.separator();
    ui.label(RichText::new(lang.tr("GPS")).strong());
    ui.vertical(|ui| {
        kv(ui, "经纬度", &format!("{:.6}, {:.6}", v.gps.lat, v.gps.lon));
        ui.horizontal(|ui| {
            kv(ui, "海拔", &format!("{:.1} m", v.gps.alt));
            kv(ui, "相对高", &format!("{:.1} m", v.gps.relative_alt));
            kv(ui, "航向", &format!("{:.0}°", v.gps.heading));
        });
        ui.horizontal(|ui| {
            let fix = match v.gps.fix_type {
                0..=1 => ("无定位", Color32::RED),
                2 => ("2D", Color32::YELLOW),
                3 => ("3D", Color32::GREEN),
                _ => ("RTK", Color32::LIGHT_BLUE),
            };
            ui.label(
                RichText::new(format!("定位: {}", fix.0))
                    .color(fix.1)
                    .strong(),
            );
            kv(ui, "星数", &format!("{}", v.gps.satellites));
        });
        ui.horizontal(|ui| {
            if v.gps.hdop > 0.0 {
                let col = if v.gps.hdop < 1.5 {
                    Color32::GREEN
                } else if v.gps.hdop < 3.0 {
                    Color32::YELLOW
                } else {
                    Color32::RED
                };
                ui.label(
                    RichText::new(format!("HDOP: {:.1} m", v.gps.hdop)).color(col),
                );
                if v.gps.vdop > 0.0 {
                    kv(ui, "VDOP", &format!("{:.1} m", v.gps.vdop));
                }
            }
            if v.gps.ground_speed > 0.0 {
                kv(ui, "地速(GPS)", &format!("{:.1} m/s", v.gps.ground_speed));
            }
        });
    });

    ui.separator();
    ui.label(RichText::new(lang.tr("电池")).strong());
    ui.horizontal(|ui| {
        kv(ui, "电压", &format!("{:.2} V", v.battery.voltage));
        kv(ui, "电流", &format!("{:.2} A", v.battery.current));
        match v.battery.remaining_pct {
            Some(p) => {
                let col = if p < 20 {
                    Color32::RED
                } else if p < 50 {
                    Color32::YELLOW
                } else {
                    Color32::GREEN
                };
                ui.label(RichText::new(format!("余电: {p}%")).color(col).strong());
            }
            None => {
                ui.label("余电: 未知");
            }
        }
    });

    ui.separator();
    if let Some(hb) = &v.heartbeat {
        ui.label(RichText::new(lang.tr("心跳")).strong());
        ui.horizontal(|ui| {
            kv(ui, "MAV type", &format!("{}", hb.mav_type));
            kv(ui, "Autopilot", &format!("{}", hb.autopilot));
            kv(ui, "Status", &format!("{}", hb.system_status));
            kv(ui, "base_mode", &format!("0x{:02X}", hb.base_mode));
        });
    }
}

/// 飞行控制区：模式切换 + 一键指令 + 回执状态
fn flight_control_section(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp, v: &VehicleModel) {
    let lang = state.lang;
    ui.label(RichText::new(lang.tr("飞行控制")).strong());

    // 模式选择行
    ui.horizontal(|ui| {
        ui.label(lang.tr("模式:"));
        let cur = v.heartbeat.as_ref().map(|h| h.custom_mode).unwrap_or(0);
        let cur_name = COPTER_MODES
            .iter()
            .find(|(m, _)| *m == cur)
            .map(|(_, n)| *n)
            .unwrap_or("自定义");
        let mut sel = cur_name;
        egui::ComboBox::from_id_source("mode_combo")
            .selected_text(sel)
            .show_ui(ui, |ui| {
                for (m, name) in COPTER_MODES {
                    ui.selectable_value(&mut sel, *name, *name)
                        .on_hover_text(format!("custom_mode={m}"));
                }
            });
        let target_mode = COPTER_MODES
            .iter()
            .find(|(_, n)| *n == sel)
            .map(|(m, _)| *m);
        if ui.button(lang.tr("切换")).clicked() {
            if let Some(m) = target_mode {
                app.send_command(FlightCommand::SetMode(m));
                state.log.push(format!("已发送模式切换 -> {sel}"));
            }
        }
    });

    // 一键指令按钮行
    ui.horizontal(|ui| {
        let armed = v.is_armed();
        if armed {
            if ui
                .button(RichText::new(format!("{} DISARM", lang.tr("上锁"))).color(Color32::YELLOW))
                .clicked()
            {
                app.send_command(FlightCommand::Disarm);
                state.log.push("已发送上锁".into());
            }
        } else if ui
            .button(RichText::new(format!("{} ARM", lang.tr("解锁"))).color(Color32::RED))
            .clicked()
        {
            app.send_command(FlightCommand::Arm);
            state.log.push("已发送解锁".into());
        }
        ui.separator();
        if ui.button(format!("{} TAKEOFF", lang.tr("起飞"))).clicked() {
            app.send_command(FlightCommand::Takeoff(10.0));
            state.log.push("已发送起飞(10m)".into());
        }
        if ui.button(format!("{} LAND", lang.tr("降落"))).clicked() {
            app.send_command(FlightCommand::Land);
            state.log.push("已发送降落".into());
        }
        if ui.button(format!("{} RTL", lang.tr("返航"))).clicked() {
            app.send_command(FlightCommand::Rtl);
            state.log.push("已发送返航".into());
        }
    });

    // 指令回执状态
    if let Some(txt) = v.ack_text() {
        let ok = v
            .cmd_ack
            .as_ref()
            .map(|a| a.result == 0)
            .unwrap_or(false);
        let col = if ok { Color32::GREEN } else { Color32::from_rgb(255, 140, 0) };
        ui.label(RichText::new(format!("{} {txt}", lang.tr("回执:"))).color(col));
    }
}

/// 遥控通道区：8 通道条形 + 信号强度
fn rc_section(ui: &mut Ui, v: &VehicleModel, lang: crate::i18n::Lang) {
    ui.label(RichText::new(lang.tr("遥控")).strong());
    let rc = &v.rc;
    if !rc.seen {
        ui.label("无遥控数据");
        return;
    }
    ui.horizontal(|ui| {
        let ok = rc.link_ok();
        ui.label(
            RichText::new(if ok { "● 遥控在线" } else { "○ 信号丢失" })
                .color(if ok { Color32::GREEN } else { Color32::RED }),
        );
        kv(ui, "通道数", &format!("{}", rc.chancount));
        if rc.rssi != 255 && rc.rssi > 0 {
            kv(ui, "信号", &format!("{}%", rc.rssi * 100 / 254));
        }
    });
    let names = ["CH1 横滚", "CH2 俯仰", "CH3 油门", "CH4 偏航", "CH5", "CH6", "CH7", "CH8"];
    for (i, name) in names.iter().enumerate() {
        let raw = rc.ch[i];
        ui.horizontal(|ui| {
        ui.add_sized(
            [70.0, 20.0],
            egui::Label::new(RichText::new(*name).weak()),
        );
            if raw == 0 {
                ui.label("-");
                return;
            }
            // 1000..2000 PWM -> 0..1 条形（1500 为中心）
            let frac = ((raw as f32 - 1000.0) / 1000.0).clamp(0.0, 1.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::new(160.0, 12.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 3.0, Color32::from_gray(50));
            if frac > 0.001 {
                let w = rect.width() * frac;
                let fill = if (0.42..=0.58).contains(&frac) {
                    Color32::GREEN
                } else if (0.3..=0.7).contains(&frac) {
                    Color32::YELLOW
                } else {
                    Color32::RED
                };
                painter.rect_filled(
                    egui::Rect::from_min_size(rect.min, egui::Vec2::new(w, rect.height())),
                    3.0,
                    fill,
                );
            }
            // 中心线
            let cx = rect.min.x + rect.width() * 0.5;
            painter.line_segment(
                [egui::pos2(cx, rect.min.y), egui::pos2(cx, rect.max.y)],
                egui::Stroke::new(1.0, Color32::WHITE),
            );
            ui.label(format!("{raw}"));
        });
    }
}

/// 键值对行（紧凑）
fn kv(ui: &mut Ui, k: &str, val: &str) {
    ui.label(RichText::new(format!("{k}: ")).weak());
    ui.label(val);
}
