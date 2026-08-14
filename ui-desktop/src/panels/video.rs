//! 视频 / OSD 面板（P3-2）：UDP MJPEG 接收、解码显示、遥测 OSD 叠加。

use egui::{Color32, FontId, Pos2, Rect, RichText, Ui};

use crate::app::GroundControlApp;
use crate::state::UiState;

pub fn video_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    let lang = state.lang;
    ui.heading(lang.tr("视频 / OSD"));
    ui.label(
        RichText::new(
            "监听 UDP MJPEG 图传流（每帧 JPEG 以 FFD8/FFD9 分帧）。\
             可用「模拟视频源」在本机生成合成画面，无需摄像头即可体验 OSD 叠加。",
        )
        .color(Color32::GRAY),
    );
    ui.separator();

    // ---- 控制区 ----
    ui.horizontal(|ui| {
        ui.label(lang.tr("监听地址:"));
        ui.add(egui::TextEdit::singleline(&mut state.video_addr).desired_width(140.0));
        if !state.video_running {
            if ui
                .add_enabled(!state.video_addr.is_empty(), egui::Button::new(lang.tr("启动接收")))
                .clicked()
            {
                app.start_video(state.video_addr.clone());
            }
        } else if ui.button(lang.tr("停止接收")).clicked() {
            app.stop_video();
        }
    });
    ui.horizontal(|ui| {
        if state.video_sim {
            if ui.button(lang.tr("停止模拟源")).clicked() {
                app.stop_video_sim();
            }
        } else if ui.button(lang.tr("启动模拟视频源")).clicked() {
            app.start_video_sim();
        }
        ui.checkbox(&mut state.video_osd, lang.tr("OSD 叠加"));
    });

    // ---- 统计与反馈 ----
    if state.video_running || state.video_stats.0 > 0 {
        let (frames, bytes) = state.video_stats;
        ui.label(format!("帧: {frames}    数据: {:.1} KB", bytes as f64 / 1024.0));
    }
    if !state.video_msg.is_empty() {
        ui.label(RichText::new(&state.video_msg).color(Color32::LIGHT_BLUE));
    }
    ui.separator();

    // ---- 拉取最新帧并解码（仅新帧） ----
    let mut new_jpeg: Option<(u64, Vec<u8>)> = None;
    {
        let ctrl = app.video.lock().unwrap();
        if let Some(rx) = &ctrl.rx {
            let st = rx.stats();
            state.video_stats = (st.frames, st.bytes);
            if let Some(f) = rx.latest_frame() {
                if f.seq != state.video_drawn_seq {
                    new_jpeg = Some((f.seq, f.jpeg));
                }
            }
        }
    }
    if let Some((seq, jpeg)) = new_jpeg {
        match image::load_from_memory(&jpeg) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = (rgba.width() as usize, rgba.height() as usize);
                let color = egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw());
                match &mut state.video_texture {
                    Some(tex) if tex.size() == [w, h] => {
                        tex.set(color, egui::TextureOptions::LINEAR);
                    }
                    _ => {
                        state.video_texture =
                            Some(ui.ctx().load_texture("video", color, egui::TextureOptions::LINEAR));
                    }
                }
                state.video_drawn_seq = seq;
            }
            Err(e) => {
                // 解码失败：保留旧帧，提示一次
                state.video_drawn_seq = seq;
                state.video_msg = format!("JPEG 解码失败: {e}");
            }
        }
    }

    // ---- 画面 + OSD ----
    if let Some(tex) = &state.video_texture {
        let avail = ui.available_size();
        let tsize = tex.size_vec2();
        let scale = (avail.x / tsize.x).min(avail.y / tsize.y).min(1.5);
        let draw = tsize * scale;
        let (rect, _) = ui.allocate_exact_size(draw, egui::Sense::hover());
        ui.painter().image(
            tex.id(),
            rect,
            egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        if state.video_osd {
            draw_osd(ui, rect, state);
        }
    } else {
        ui.label(
            RichText::new("（无视频画面：启动接收并发送 MJPEG 帧，或点击「启动模拟视频源」）")
                .color(Color32::GRAY),
        );
    }
}

/// 在视频画面上叠加遥测 OSD（模式/高度/速度/电量/GPS/姿态）
fn draw_osd(ui: &Ui, rect: Rect, state: &UiState) {
    let v = &state.vehicle;
    let p = ui.painter();
    let font = FontId::monospace(13.0);
    let shade = Color32::from_black_alpha(110);
    let txt = Color32::WHITE;
    let pad = 6.0;

    // 单行 OSD 文本，带半透明底
    let line = |pos: Pos2, text: String| {
        let galley = p.layout_no_wrap(text, font.clone(), txt);
        let size = galley.size();
        p.rect_filled(
            Rect::from_min_size(pos, size + egui::vec2(pad * 2.0, pad)),
            2.0,
            shade,
        );
        p.galley(pos + egui::vec2(pad, pad * 0.5), galley, txt);
    };

    // 左上：模式 / 系统状态
    let mut y = rect.top() + 4.0;
    line(Pos2::new(rect.left() + 4.0, y), format!("模式: {}", v.flight_mode_name()));
    y += 22.0;
    if let Some(h) = &v.heartbeat {
        line(Pos2::new(rect.left() + 4.0, y), format!("状态: {}", state_name(h.system_status)));
        y += 22.0;
    }
    line(
        Pos2::new(rect.left() + 4.0, y),
        format!("高度: {:.1} m", v.gps.relative_alt),
    );
    y += 22.0;
    line(
        Pos2::new(rect.left() + 4.0, y),
        format!("速度: {:.1} m/s", v.air.groundspeed),
    );
    y += 22.0;
    line(
        Pos2::new(rect.left() + 4.0, y),
        format!(
            "电池: {:.1} V {}%",
            v.battery.voltage,
            v.battery.remaining_pct.unwrap_or(0)
        ),
    );

    // 右上：GPS
    if v.gps.lat != 0.0 || v.gps.lon != 0.0 {
        let gps = format!(
            "GPS {:.6}, {:.6}  星{}",
            v.gps.lat, v.gps.lon, v.gps.satellites
        );
        let galley = p.layout_no_wrap(gps, font.clone(), txt);
        let w = galley.size().x + pad * 2.0;
        let pos = Pos2::new(rect.right() - w - 4.0, rect.top() + 4.0);
        p.rect_filled(
            Rect::from_min_size(pos, galley.size() + egui::vec2(pad * 2.0, pad)),
            2.0,
            shade,
        );
        p.galley(pos + egui::vec2(pad, pad * 0.5), galley, txt);
    }

    // 左下：姿态
    let att = format!(
        "横滚 {:+.0}°  俯仰 {:+.0}°  航向 {:+.0}°",
        v.attitude.roll.to_degrees(),
        v.attitude.pitch.to_degrees(),
        v.attitude.yaw.to_degrees(),
    );
    let galley = p.layout_no_wrap(att, font.clone(), txt);
    let pos = Pos2::new(rect.left() + 4.0, rect.bottom() - 22.0);
    p.rect_filled(
        Rect::from_min_size(pos, galley.size() + egui::vec2(pad * 2.0, pad)),
        2.0,
        shade,
    );
    p.galley(pos + egui::vec2(pad, pad * 0.5), galley, txt);

    // 中央：航向指示（简化横线 + 中间刻度）
    let mid_y = rect.center().y;
    p.line_segment(
        [Pos2::new(rect.left() + 30.0, mid_y), Pos2::new(rect.right() - 30.0, mid_y)],
        egui::Stroke::new(1.5_f32, Color32::from_white_alpha(140)),
    );
    p.circle_filled(rect.center(), 3.0, Color32::from_rgb(255, 140, 0));
    let yaw_txt = p.layout_no_wrap(
        format!("{:03.0}°", v.attitude.yaw.to_degrees().rem_euclid(360.0)),
        FontId::monospace(12.0),
        Color32::from_rgb(255, 220, 120),
    );
    p.galley(
        Pos2::new(rect.center().x - yaw_txt.size().x / 2.0, mid_y + 8.0),
        yaw_txt,
        Color32::WHITE,
    );
}

/// MAV_STATE 可读名
fn state_name(s: u8) -> &'static str {
    match s {
        0 => "未初始化",
        1 => "引导中",
        2 => "校准中",
        3 => "待机",
        4 => "运行中",
        5 => "紧急",
        6 => "保护关断",
        7 => "回收中",
        _ => "未知",
    }
}
