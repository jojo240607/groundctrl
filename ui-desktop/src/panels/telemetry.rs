//! 遥测面板：姿态、人工地平仪、空速/高度仪表、GPS、电池、心跳。

use egui::{Color32, RichText, Ui};

use groundctrl_core::vehicle::VehicleModel;

use crate::widgets::horizon::attitude_indicator;
use crate::widgets::round_gauge::round_gauge;

pub fn telemetry_panel(ui: &mut Ui, v: &VehicleModel) {
    ui.heading("遥测");

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
            let armed = hb.base_mode & 0x80 != 0;
            ui.label(RichText::new(if armed { "ARMED" } else { "DISARMED" }).color(
                if armed { Color32::RED } else { Color32::YELLOW },
            ));
        }
    });

    ui.separator();
    ui.label(RichText::new("姿态").strong());
    ui.horizontal(|ui| {
        attitude_indicator(ui, v, 160.0);
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
    ui.label(RichText::new("GPS").strong());
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
    });

    ui.separator();
    ui.label(RichText::new("电池").strong());
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
        ui.label(RichText::new("心跳").strong());
        ui.horizontal(|ui| {
            kv(ui, "MAV type", &format!("{}", hb.mav_type));
            kv(ui, "Autopilot", &format!("{}", hb.autopilot));
            kv(ui, "Status", &format!("{}", hb.system_status));
            kv(ui, "base_mode", &format!("0x{:02X}", hb.base_mode));
        });
    }
}

/// 键值对行（紧凑）
fn kv(ui: &mut Ui, k: &str, val: &str) {
    ui.label(RichText::new(format!("{k}: ")).weak());
    ui.label(val);
}
