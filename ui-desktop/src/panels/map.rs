//! 地图面板：离线瓦片（{z}/{x}/{y}.png）+ 航迹/航点/指北针叠加。

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::app::GroundControlApp;
use crate::state::UiState;
use groundctrl_core::vehicle::mission::Waypoint;
use crate::widgets::tiles::{
    fetch_tile, lat2ytile, load_tile_texture, lon2xtile, merc_y2lat, tile_path,
};

pub fn map_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("地图 (在线/离线瓦片)");

    // 机队选择：下拉列出所有在网飞机
    let fleet: Vec<u8> = state.vehicles.keys().cloned().collect();
    if !fleet.is_empty() {
        ui.horizontal(|ui| {
            let sel = state.selected_sys.unwrap_or(*fleet.last().unwrap_or(&0));
            ui.label("飞机:");
            egui::ComboBox::from_id_source("map_fleet_sel")
                .selected_text(format!("SYS {}", sel))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.selected_sys, Some(*fleet.last().unwrap_or(&0)), "最新 (自动)");
                    for s in &fleet {
                        ui.selectable_value(&mut state.selected_sys, Some(*s), format!("SYS {}", s));
                    }
                });
        });
    }

    ui.horizontal(|ui| {
        ui.checkbox(&mut state.online_tiles, "在线地图");
        ui.checkbox(&mut state.map_click_add_wp, "点击地图添加航点");
        if ui.button("放大").clicked() {
            state.map_zoom = (state.map_zoom * 1.4).min(18.0);
        }
        if ui.button("缩小").clicked() {
            state.map_zoom = (state.map_zoom / 1.4).max(2.0);
        }
        ui.label(format!("zoom {:.1}", state.map_zoom));
        ui.label("缩放:");
        ui.add(egui::Slider::new(&mut state.map_zoom, 2.0..=18.0).logarithmic(true));
        ui.label(format!("z={:.1}", state.map_zoom));
        if ui.button("选择瓦片目录...").clicked() {
            let rt = app.rt.handle().clone();
            let st = app.state.clone();
            let h = app.save_handle();
            rt.spawn(async move {
                if let Some(dir) = rfd::AsyncFileDialog::new()
                    .set_title("选择瓦片根目录 ({z}/{x}/{y}.png)")
                    .pick_folder()
                    .await
                {
                    if let Ok(mut s) = st.lock() {
                        s.tile_dir = Some(dir.path().to_path_buf());
                        s.tile_cache.clear(); // 切换目录时清空缓存
                        drop(s);
                        h.save();
                    }
                }
            });
        }
        if let Some(d) = &state.tile_dir {
            let _ = d; // 显示路径会很长，仅提示已加载
            ui.label("● 离线瓦片目录");
        } else {
            ui.label("○ 在线缓存模式");
        }
    });
    let ctx = ui.ctx().clone();
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let rect = resp.rect;

    let trail = state
        .selected_sys
        .and_then(|s| state.trails.get(&s).cloned())
        .or_else(|| state.trails.values().next().cloned())
        .unwrap_or_default();

    if trail.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "无 GPS 轨迹",
            FontId::default(),
            Color32::GRAY,
        );
        return;
    }

    // 以当前位置为视图中心；zoom 越大视野越窄
    let cur = *trail.last().unwrap();
    let half_span = (1.0 / (state.map_zoom * state.map_zoom)) + 0.0008;
    let min_lat = cur.0 - half_span;
    let max_lat = cur.0 + half_span;
    let min_lon = cur.1 - half_span;
    let max_lon = cur.1 + half_span;
    let lat_span = (max_lat - min_lat).max(1e-9);
    let lon_span = (max_lon - min_lon).max(1e-9);

    let pad: f64 = 16.0;
    let w: f64 = (rect.width() - 2.0 * pad as f32) as f64;
    let h: f64 = (rect.height() - 2.0 * pad as f32) as f64;

    let to_xy = |la: f64, lo: f64| -> Pos2 {
        let x = pad + ((lo - min_lon) / lon_span) * w;
        let y = pad + ((max_lat - la) / lat_span) * h;
        Pos2::new(rect.min.x + x as f32, rect.min.y + y as f32)
    };

    // 反算屏幕坐标 -> 经纬度
    let from_xy = |pos: Pos2| -> (f64, f64) {
        let lx = (pos.x - rect.min.x - pad as f32) as f64;
        let ly = (pos.y - rect.min.y - pad as f32) as f64;
        let lo = min_lon + (lx / w) * lon_span;
        let la = max_lat - (ly / h) * lat_span;
        (la.clamp(-90.0, 90.0), lo)
    };

    // 航点交互：选中后可拖拽移动；空白点击（添加模式）则新增
    if let Some(pos) = resp.hover_pos() {
        let hit = state
            .mission
            .iter()
            .enumerate()
            .find(|(_, wp)| to_xy(wp.lat, wp.lon).distance(pos) < 10.0)
            .map(|(i, _)| i);

        if resp.drag_started() {
            if let Some(i) = hit {
                state.dragging_wp = Some(i);
            } else if state.map_click_add_wp {
                let (la, lo) = from_xy(pos);
                state.mission.push(Waypoint::nav(la, lo, state.wp_alt));
            }
        }
        if resp.dragged() {
            if let Some(i) = state.dragging_wp {
                if i < state.mission.len() {
                    let (la, lo) = from_xy(pos);
                    state.mission[i].lat = la;
                    state.mission[i].lon = lo;
                }
            }
        }
        if resp.drag_released() {
            state.dragging_wp = None;
        }
    }

    // 瓦片底图：优先用户目录，否则默认在线缓存目录（开箱即用）
    if let Some(root) = state.tile_root() {
        let z = state.map_zoom.round() as i32;
        let n = 2f64.powi(z);
        let x0 = lon2xtile(min_lon, n).floor() as i32;
        let x1 = lon2xtile(max_lon, n).floor() as i32;
        let y0 = lat2ytile(max_lat, n).floor() as i32;
        let y1 = lat2ytile(min_lat, n).floor() as i32;
        for tx in x0..=x1 {
            for ty in y0..=y1 {
                if tx < 0 || ty < 0 || tx >= n as i32 || ty >= n as i32 {
                    continue;
                }
                let key = format!("{z}/{tx}/{ty}");
                let path = tile_path(&root, z, tx, ty);
                let tex = if let Some(t) = state.tile_cache.get(&key) {
                    t.clone()
                } else if path.exists() {
                    // 磁盘已有（之前下载或离线目录）：加载到纹理缓存
                    let img = load_tile_texture(&ctx, &path, &key);
                    state.tile_cache.insert(key.clone(), img.clone());
                    img
                } else {
                    // 缺失：若开启在线地图，后台下载一次（避免重复触发）
                    if state.online_tiles && !state.pending_tiles.contains(&key) {
                        state.pending_tiles.insert(key.clone());
                        let rt = app.rt.handle().clone();
                        let st = app.state.clone();
                        let cache_dir = root.clone();
                        let url_tmpl = state.tile_url.clone();
                        let key_c = key.clone();
                        rt.spawn(async move {
                            let ok = fetch_tile(z, tx, ty, &cache_dir, &url_tmpl).await;
                            // 下载完成（无论成败）解除 pending，下一帧重绘
                            if let Ok(mut s) = st.lock() {
                                s.pending_tiles.remove(&key_c);
                                if ok {
                                    s.tile_cache.remove(&key_c); // 强制下一帧从磁盘加载
                                }
                            }
                        });
                    }
                    // 暂用灰占位纹理
                    load_tile_texture(&ctx, &path, &key)
                };
                // 瓦片四角经纬度 -> 屏幕坐标
                let t_min_lon = tx as f64 / n * 360.0 - 180.0;
                let t_max_lat = merc_y2lat((ty as f64) / n);
                let t_max_lon = (tx as f64 + 1.0) / n * 360.0 - 180.0;
                let t_min_lat = merc_y2lat((ty as f64 + 1.0) / n);
                let p_tl = to_xy(t_max_lat, t_min_lon);
                let p_br = to_xy(t_min_lat, t_max_lon);
                let r = Rect::from_two_pos(p_tl, p_br);
                ui.painter().image(
                    tex.id(),
                    r,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
    }

    draw_overlays(&painter, &rect, &to_xy, &trail, &state.mission);
}

/// 航迹线、航点、当前点、指北针叠加
fn draw_overlays(
    painter: &Painter,
    rect: &Rect,
    to_xy: &dyn Fn(f64, f64) -> Pos2,
    trail: &[(f64, f64)],
    mission: &[groundctrl_core::vehicle::mission::Waypoint],
) {
    // 航迹线
    let pts: Vec<Pos2> = trail.iter().map(|(la, lo)| to_xy(*la, *lo)).collect();
    for seg in pts.windows(2) {
        painter.line_segment(
            [seg[0], seg[1]],
            Stroke::new(1.5_f32, Color32::LIGHT_BLUE),
        );
    }

    // 本地航点叠加（橙色方块）
    for wp in mission {
        let p = to_xy(wp.lat, wp.lon);
        painter.rect_filled(
            Rect::from_center_size(p, Vec2::splat(8.0)),
            1.0,
            Color32::GOLD,
        );
    }

    // 当前点
    if let Some(&cur_p) = pts.last() {
        painter.circle_filled(cur_p, 4.0, Color32::GREEN);
    }

    // 指北针（左上角小指示器）
    let n_size = 26.0;
    let n_center = rect.min + Vec2::new(n_size + 10.0, n_size + 10.0);
    painter.circle_stroke(n_center, n_size, Stroke::new(1.0_f32, Color32::GRAY));
    painter.line_segment(
        [n_center, n_center - Vec2::new(0.0, n_size)],
        Stroke::new(2.0_f32, Color32::RED),
    );
    painter.text(
        n_center - Vec2::new(0.0, n_size + 9.0),
        Align2::CENTER_CENTER,
        "N",
        FontId::proportional(12.0),
        Color32::RED,
    );
}
