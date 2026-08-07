//! 遥测面板：姿态、人工地平仪、空速/高度仪表、GPS、电池、心跳。

use egui::Ui;

use groundctrl_core::vehicle::VehicleModel;

use crate::widgets::horizon::attitude_indicator;
use crate::widgets::round_gauge::round_gauge;

pub fn telemetry_panel(ui: &mut Ui, v: &VehicleModel) {
    ui.heading("遥测");
    ui.horizontal(|ui| {
        ui.label("在线:");
        ui.label(if v.online { "YES" } else { "NO" });
        ui.label(format!("sys={} comp={}", v.sys_id, v.comp_id));
    });

    ui.separator();
    ui.label("姿态 (deg)");
    ui.label(format!(
        "roll: {:.1}  pitch: {:.1}  yaw: {:.1}",
        v.attitude.roll.to_degrees(),
        v.attitude.pitch.to_degrees(),
        v.attitude.yaw.to_degrees()
    ));

    // 人工地平仪（artificial horizon）
    attitude_indicator(ui, v, 160.0);
    ui.label(format!(
        "roll: {:.0}°  pitch: {:.0}°  yaw: {:.0}°",
        v.attitude.roll.to_degrees(),
        v.attitude.pitch.to_degrees(),
        v.attitude.yaw.to_degrees()
    ));

    ui.separator();
    // 空速表 + 高度表（圆形仪表，指针式）
    ui.label("空速 / 高度");
    ui.horizontal(|ui| {
        let (resp_a, painter_a) = ui.allocate_painter(egui::Vec2::new(150.0, 150.0), egui::Sense::hover());
        round_gauge(
            &painter_a,
            resp_a.rect.center(),
            66.0,
            v.air.airspeed,
            "空速 m/s",
            0.0,
            40.0,
        );
        let (resp_h, painter_h) = ui.allocate_painter(egui::Vec2::new(150.0, 150.0), egui::Sense::hover());
        round_gauge(
            &painter_h,
            resp_h.rect.center(),
            66.0,
            v.gps.relative_alt,
            "高度 m",
            0.0,
            120.0,
        );
    });
    ui.label(format!(
        "地速: {:.1} m/s   爬升: {:.1} m/s   油门: {}%",
        v.air.groundspeed, v.air.climb, v.air.throttle
    ));

    ui.separator();
    ui.label("GPS");
    ui.label(format!("lat: {:.6}  lon: {:.6}", v.gps.lat, v.gps.lon));
    ui.label(format!(
        "alt: {:.1} m  rel: {:.1} m  hdg: {:.0}°",
        v.gps.alt, v.gps.relative_alt, v.gps.heading
    ));
    ui.label(format!("fix: {}  sats: {}", v.gps.fix_type, v.gps.satellites));

    ui.separator();
    ui.label("电池");
    ui.label(format!(
        "V: {:.2}  I: {:.2} A  rem: {}",
        v.battery.voltage,
        v.battery.current,
        v.battery
            .remaining_pct
            .map(|p| format!("{p}%"))
            .unwrap_or_else(|| "未知".to_string())
    ));

    ui.separator();
    if let Some(hb) = &v.heartbeat {
        ui.label(format!(
            "HB: type={} ap={} status={} base_mode=0x{:02X}",
            hb.mav_type, hb.autopilot, hb.system_status, hb.base_mode
        ));
    }
}
