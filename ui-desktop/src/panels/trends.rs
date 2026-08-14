//! 参数趋势面板：实时参数折线图（1Hz 采样）+ tlog 日志图表分析（P2-1）。

use egui::{Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui};

use groundctrl_core::services::log::LogManager;

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
    } else {
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
        let (resp, painter) = ui.allocate_painter(
            ui.available_size().min(egui::Vec2::new(2000.0, 260.0)),
            Sense::hover(),
        );
        let series: Vec<(String, Vec<(f64, f64)>)> = state
            .trend_selected
            .iter()
            .filter_map(|n| {
                state
                    .param_trends
                    .get(n)
                    .map(|s| (n.clone(), s.clone()))
            })
            .collect();
        draw_series_lines(&painter, resp.rect, &series, "等待采样数据...");
    }

    ui.separator();
    log_analysis_section(ui, state, app);
}

/// tlog 日志图表分析区：加载 tlog → 字段多选 → 绘制整段飞行曲线
fn log_analysis_section(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("tlog 图表分析");
    ui.horizontal(|ui| {
        if ui.button("加载 tlog 分析...").clicked() {
            analyze_tlog(state, app);
        }
        if !state.log_series.is_empty() {
            if ui.button("全选").clicked() {
                state.log_series_sel = (0..state.log_series.len().min(PALETTE.len()))
                    .collect();
            }
            if ui.button("清空").clicked() {
                state.log_series_sel.clear();
            }
        }
    });
    if !state.log_series_msg.is_empty() {
        ui.label(RichText::new(&state.log_series_msg).color(egui::Color32::LIGHT_BLUE));
    }

    if state.log_series.is_empty() {
        ui.label("加载录制的 tlog 文件后，可绘制高度 / 速度 / 电量 / 姿态等字段随时间的变化曲线。");
        return;
    }

    // 字段多选
    ui.horizontal_wrapped(|ui| {
        for (i, s) in state.log_series.iter().enumerate() {
            let selected = state.log_series_sel.contains(&i);
            let mut check = selected;
            if ui
                .checkbox(&mut check, format!("{} ({})", s.name, s.unit))
                .changed()
            {
                if check {
                    if state.log_series_sel.len() < PALETTE.len() {
                        state.log_series_sel.push(i);
                    }
                } else {
                    state.log_series_sel.retain(|x| *x != i);
                }
            }
        }
    });

    let (resp, painter) = ui.allocate_painter(
        ui.available_size().min(egui::Vec2::new(2000.0, 360.0)),
        Sense::hover(),
    );
    let series: Vec<(String, Vec<(f64, f64)>)> = state
        .log_series_sel
        .iter()
        .filter_map(|&i| {
            state
                .log_series
                .get(i)
                .map(|s| (format!("{} ({})", s.name, s.unit), s.points.clone()))
        })
        .collect();
    draw_series_lines(&painter, resp.rect, &series, "勾选上方字段以绘制曲线");
}

/// 异步加载 tlog 并提取序列
fn analyze_tlog(state: &mut UiState, app: &GroundControlApp) {
    let rt = app.rt.handle().clone();
    let st = app.state.clone();
    let _ = state;
    rt.spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_title("选择 tlog 日志进行分析")
            .add_filter("tlog", &["tlog"])
            .pick_file()
            .await
        {
            let sp = path
                .path()
                .to_str()
                .unwrap_or("flight.tlog")
                .to_string();
            let (series, msg) = match LogManager::load_file(&sp) {
                Ok(lm) => {
                    let s = lm.analyze();
                    let names: Vec<String> =
                        s.iter().map(|x| x.name.clone()).collect();
                    let msg = format!(
                        "已分析 {} 帧 → {} 条序列: {}",
                        lm.len(),
                        s.len(),
                        names.join(" / ")
                    );
                    (s, msg)
                }
                Err(e) => (Vec::new(), format!("加载失败: {e}")),
            };
            if let Ok(mut st) = st.lock() {
                st.log_series = series;
                st.log_series_sel = (0..st.log_series.len().min(PALETTE.len()))
                    .collect();
                st.log_series_msg = msg;
            }
        }
    });
}

/// 通用多序列折线绘制：x=相对时间(秒)，y=数值，各序列共享坐标范围
fn draw_series_lines(
    painter: &egui::Painter,
    rect: Rect,
    series: &[(String, Vec<(f64, f64)>)],
    empty_msg: &str,
) {
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
    for (_n, pts) in series {
        for &(t, v) in pts {
            any = true;
            t_min = t_min.min(t);
            t_max = t_max.max(t);
            v_min = v_min.min(v);
            v_max = v_max.max(v);
        }
    }

    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, Color32::DARK_GRAY));

    if !any || t_max - t_min < 1e-6 {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            empty_msg,
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
    // 时间轴刻度（0s 与末尾）
    painter.text(
        Pos2::new(plot.min.x, plot.max.y),
        egui::Align2::LEFT_TOP,
        "0s",
        FontId::default(),
        Color32::GRAY,
    );
    painter.text(
        Pos2::new(plot.max.x, plot.max.y),
        egui::Align2::RIGHT_TOP,
        format!("{t_max:.0}s"),
        FontId::default(),
        Color32::GRAY,
    );

    for (i, (_n, pts)) in series.iter().enumerate() {
        let c = PALETTE[i % PALETTE.len()];
        if pts.len() < 2 {
            continue;
        }
        let p: Vec<Pos2> = pts.iter().map(|&(t, v)| to_xy(t, v)).collect();
        painter.add(egui::Shape::line(p, Stroke::new(1.5_f32, c)));
    }
}
