//! 人工地平仪（姿态仪）：天空/地面填充 + 俯仰刻度 + 固定机体符号。

use egui::{
    Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2,
};

use groundctrl_core::vehicle::VehicleModel;

/// 在给定 `ui` 上分配 `size × size` 画布并绘制人工地平仪。
/// roll/pitch 取 `v.attitude`（弧度），每度约 2px 俯仰偏移。
pub fn attitude_indicator(ui: &mut Ui, v: &VehicleModel, size: f32) {
    let (resp, mut painter) = ui.allocate_painter(Vec2::new(size, size), Sense::hover());
    let center = resp.rect.center();
    let r = size / 2.0 - 4.0;

    // 限制极端角度，避免几何发散
    let roll = v.attitude.roll;
    let pitch = v
        .attitude
        .pitch
        .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);

    // 俯仰在屏幕上偏移：每度约 2px
    let pitch_px = pitch.to_degrees() * 2.0;
    let cs = roll.cos() as f32;
    let sn = roll.sin() as f32;

    // 地平线在旋转坐标系下的 y 偏移（未旋转前）
    let horizon_y = pitch_px;
    // 旋转后的方向向量：屏幕 x 轴在机体坐标系下
    let rot = |x: f32, y: f32| -> Pos2 {
        Pos2::new(center.x + (x * cs - y * sn), center.y + (x * sn + y * cs))
    };

    // 裁剪到圆
    let clip_rect = Rect::from_center_size(center, Vec2::splat(size));
    painter.set_clip_rect(clip_rect);

    // 天空 / 地面填充（以地平线为界，整块涂色后旋转）
    let far = r + 60.0;
    let sky = vec![
        rot(-far, -far + horizon_y),
        rot(far, -far + horizon_y),
        rot(far, -far),
        rot(-far, -far),
    ];
    painter.add(Shape::convex_polygon(
        sky,
        Color32::from_rgb(70, 130, 200),
        Stroke::NONE,
    ));
    let ground = vec![
        rot(-far, far + horizon_y),
        rot(far, far + horizon_y),
        rot(far, far),
        rot(-far, far),
    ];
    painter.add(Shape::convex_polygon(
        ground,
        Color32::from_rgb(120, 90, 50),
        Stroke::NONE,
    ));

    // 地平线（白线）
    let hl_a = rot(-r, horizon_y);
    let hl_b = rot(r, horizon_y);
    painter.line_segment([hl_a, hl_b], Stroke::new(2.0_f32, Color32::WHITE));

    // 俯仰刻度（每 10° 一条，带标号）
    for deg in [-30, -20, -10, 10, 20, 30] {
        let y = horizon_y - (deg as f32) * 2.0;
        let len = if deg % 20 == 0 { 24.0 } else { 14.0 };
        let a = rot(-len, y);
        let b = rot(len, y);
        painter.line_segment([a, b], Stroke::new(1.0_f32, Color32::WHITE));
        if deg % 20 == 0 {
            let tx = rot(len + 6.0, y);
            painter.text(
                tx,
                Align2::LEFT_CENTER,
                format!("{deg}"),
                FontId::proportional(10.0),
                Color32::WHITE,
            );
        }
    }

    // 恢复裁剪（球体描边在裁剪外画）
    painter.set_clip_rect(Rect::EVERYTHING);

    // 外圆 + 固定机体符号（不随姿态旋转）
    painter.circle_stroke(center, r, Stroke::new(1.5_f32, Color32::GRAY));
    painter.line_segment(
        [center - Vec2::new(20.0, 0.0), center - Vec2::new(6.0, 0.0)],
        Stroke::new(2.5_f32, Color32::YELLOW),
    );
    painter.line_segment(
        [center + Vec2::new(6.0, 0.0), center + Vec2::new(20.0, 0.0)],
        Stroke::new(2.5_f32, Color32::YELLOW),
    );
    painter.line_segment(
        [center, center - Vec2::new(0.0, 8.0)],
        Stroke::new(2.5_f32, Color32::YELLOW),
    );
}
