//! 圆形指针式仪表：value 映射到 [min,max] 的 270° 弧，指针 + 数字。

use egui::{Align2, Color32, FontId, Painter, Pos2, Stroke, Vec2};

/// 绘制一个圆形仪表。
///
/// `start_deg = -225`（左下）顺时针 `sweep_deg = 270`（右下），
/// 绿色弧按 `frac` 比例显示，黄色指针指向当前值，中央显示数值，底部显示标签。
pub fn round_gauge(
    painter: &Painter,
    center: Pos2,
    r: f32,
    value: f32,
    label: &str,
    min: f32,
    max: f32,
) {
    let start_deg = -225.0_f32;
    let sweep_deg = 270.0_f32;
    let frac = ((value - min) / (max - min)).clamp(0.0, 1.0);
    let val_deg = start_deg + frac * sweep_deg;

    // 表盘外圈
    painter.circle_stroke(center, r, Stroke::new(2.0_f32, Color32::GRAY));

    // 刻度弧（绿色，按 frac 比例）
    let arc_len = 40; // 分段数
    for i in 0..arc_len {
        let a0 = (start_deg + (i as f32 / arc_len as f32) * sweep_deg).to_radians();
        let a1 = (start_deg + ((i + 1) as f32 / arc_len as f32) * sweep_deg).to_radians();
        let p0 = center + Vec2::new(a0.cos(), a0.sin()) * r;
        let p1 = center + Vec2::new(a1.cos(), a1.sin()) * r;
        let col = if (i as f32 / arc_len as f32) <= frac {
            Color32::GREEN
        } else {
            Color32::DARK_GRAY
        };
        painter.line_segment([p0, p1], Stroke::new(3.0_f32, col));
    }

    // 指针
    let va = val_deg.to_radians();
    let tip = center + Vec2::new(va.cos(), va.sin()) * (r - 10.0);
    painter.line_segment([center, tip], Stroke::new(2.5_f32, Color32::YELLOW));
    painter.circle_filled(center, 4.0, Color32::YELLOW);

    // 中央数字
    painter.text(
        center + Vec2::new(0.0, r * 0.45),
        Align2::CENTER_CENTER,
        format!("{value:.1}"),
        FontId::proportional(16.0),
        Color32::WHITE,
    );
    // 标签（下方）
    painter.text(
        center + Vec2::new(0.0, r + 10.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.0),
        Color32::LIGHT_GRAY,
    );
}
