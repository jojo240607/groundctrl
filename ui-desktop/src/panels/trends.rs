//! 参数趋势面板：对选中的参数绘制实时折线图（1Hz 采样）。

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};

use crate::app::GroundControlApp;
use crate::state::UiState;

const PALETTE: [Color32; 8] = [
    Color32::RED,
    Color32::GREEN,
    Color32::BLUE,
    Color32::YELLOW,
    Color32::from_rgb(255, 128, 0),
    Color32::from_rgb(0, 200, 200),
    Color32::from_rgb(200, 0, 200),
    Color32::LIGHT_GRAY,
];

pub fn trends_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("参数趋势");

    ui.horizontal(|ui| {
        if ui.checkbox(&mut state.trend_enabled, "采样 (1Hz)").changed() {
            app.save_settings();
        }
        if !state.params.is_empty() {
            egui::ComboBox::from_id_source("trend_add")
                .selected_text("+ 添加参数")
                .show_ui(ui, |ui| {
                    let names: Vec<String> =
                        state.params.iter().map(|p| p.name.clone()).collect();
                    for n in names {
                        let already = state.trend_selected.contains(&n);
                        if ui.selectable_label(already, &n).clicked() {
                            if already {
                                state.trend_selected.retain(|x| x != &n);
                            } else if state.trend_selected.len() < PALETTE.len() {
                                state.trend_selected.push(n);
                            }
                            app.save_settings();
                        }
                    }
                });
        }
        if ui.button("清空选择").clicked() {
            state.trend_selected.clear();
            app.save_settings();
        }
    });

    if state.trend_selected.is_empty() {
        ui.label("勾选「采样」并从上方选择参数以绘制趋势曲线。");
        return;
    }

    // 图例（收集待删除项，循环结束后再 retain 避免借用冲突）
    let mut to_remove: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        for (i, n) in state.trend_selected.iter().enumerate() {
            let c = PALETTE[i % PALETTE.len()];
            ui.colored_label(c, n);
            if ui.small_button("✕").clicked() {
                to_remove = Some(n.clone());
            }
            ui.separator();
        }
    });
    if let Some(r) = to_remove {
        state.trend_selected.retain(|x| x != &r);
        app.save_settings();
    }

    let (resp, painter) =
        ui.allocate_painter(ui.available_size().min(egui::Vec2::new(2000.0, 320.0)), Sense::hover());
    let rect = resp.rect;
    draw_trends(&painter, rect, state);
}

fn draw_trends(painter: &egui::Painter, rect: Rect, state: &UiState) {
    let pad = 8.0;
    let plot = Rect::from_min_max(
        Pos2::new(rect.min.x + pad, rect.min.y + pad),
        Pos2::new(rect.max.x - pad, rect.max.y - pad),
    );

    // 计算所有选中序列的整体时间/值域
    let mut t_min = f64::MAX;
    let mut t_max = f64::MIN;
    let mut v_min = f64::MAX;
    let mut v_max = f64::MIN;
    let mut any = false;
    for n in &state.trend_selected {
        if let Some(series) = state.param_trends.get(n) {
            for &(t, v) in series {
                any = true;
                t_min = t_min.min(t);
                t_max = t_max.max(t);
                v_min = v_min.min(v);
                v_max = v_max.max(v);
            }
        }
    }

    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, Color32::DARK_GRAY));

    if !any || t_max - t_min < 1e-6 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "等待采样数据...",
            FontId::default(),
            Color32::GRAY,
        );
        return;
    }

    // 纵向留白 10%
    let v_span = (v_max - v_min).max(1e-6);
    let v_lo = v_min - v_span * 0.1;
    let v_hi = v_max + v_span * 0.1;
    let t_span = (t_max - t_min).max(1e-6);

    let to_xy = |t: f64, v: f64| -> Pos2 {
        let x = plot.min.x + (((t - t_min) / t_span) as f32) * plot.width();
        let y = plot.max.y - (((v - v_lo) / (v_hi - v_lo)) as f32) * plot.height();
        Pos2::new(x, y)
    };

    // 网格 + 轴标签
    for k in 0..=4 {
        let vy = v_lo + (v_hi - v_lo) * (k as f64 / 4.0);
        let y = to_xy(t_min, vy).y;
        painter.line_segment(
            [Pos2::new(plot.min.x, y), Pos2::new(plot.max.x, y)],
            Stroke::new(0.5_f32, Color32::DARK_GRAY),
        );
        painter.text(
            Pos2::new(plot.min.x, y),
            egui::Align2::LEFT_BOTTOM,
            format!("{vy:.2}"),
            FontId::default(),
            Color32::GRAY,
        );
    }

    for (i, n) in state.trend_selected.iter().enumerate() {
        let c = PALETTE[i % PALETTE.len()];
        if let Some(series) = state.param_trends.get(n) {
            if series.len() < 2 {
                continue;
            }
            let pts: Vec<Pos2> = series.iter().map(|&(t, v)| to_xy(t, v)).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(1.5_f32, c)));
        }
    }
}
