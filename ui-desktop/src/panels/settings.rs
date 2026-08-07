//! 设置面板：编辑连接默认值、地图/瓦片源、窗口偏好，并落盘到本地 JSON。

use egui::{Slider, Ui};

use crate::app::{GroundControlApp, TabKind};
use crate::state::UiState;

pub fn settings_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("设置");
    ui.label("修改后点击「保存到磁盘」。窗口尺寸会自动记录。");

    ui.separator();
    ui.collapsing("连接默认值", |ui| {
        ui.horizontal(|ui| {
            ui.label("串口:");
            ui.text_edit_singleline(&mut state.serial_port);
        });
        ui.add(Slider::new(&mut state.baud, 9600..=921600).logarithmic(true).text("波特率"));
        ui.horizontal(|ui| {
            ui.label("UDP bind:");
            ui.text_edit_singleline(&mut state.udp_bind);
        });
        ui.horizontal(|ui| {
            ui.label("UDP target:");
            ui.text_edit_singleline(&mut state.udp_target);
        });
    });

    ui.separator();
    ui.collapsing("地图 / 瓦片", |ui| {
        ui.checkbox(&mut state.online_tiles, "启用在线瓦片下载 (开箱即用)");
        ui.horizontal(|ui| {
            ui.label("瓦片源 URL:");
            ui.text_edit_singleline(&mut state.tile_url);
        });
        ui.add(Slider::new(&mut state.map_zoom, 2.0..=18.0).logarithmic(true).text("默认缩放"));
        ui.horizontal(|ui| {
            if let Some(d) = &state.tile_dir {
                ui.label(format!("离线瓦片目录: {}", d.display()));
            } else {
                ui.label("瓦片目录: (使用默认在线缓存目录)");
            }
            if ui.button("清除离线目录").clicked() {
                state.tile_dir = None;
                state.tile_cache.clear();
            }
            if ui.button("选择目录...").clicked() {
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
                            s.tile_cache.clear();
                            drop(s);
                            h.save();
                        }
                    }
                });
            }
        });
        ui.label("URL 模板支持 {z}/{x}/{y} 占位符，例如 https://tile.openstreetmap.org/{z}/{x}/{y}.png");
    });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("保存到磁盘").clicked() {
            app.save_settings();
            state.export_msg = "设置已保存".to_string();
        }
        if ui.button("重置为默认").clicked() {
            *state = UiState::default();
            app.save_settings();
            state.export_msg = "已重置为默认设置并保存".to_string();
        }
    });

    if !state.export_msg.is_empty() {
        ui.label(&state.export_msg);
    }

    ui.separator();
    ui.label(format!("当前标签页: {:?}", state.tab));
    // 提供快速跳转到其他面板
    ui.horizontal_wrapped(|ui| {
        for (tk, label) in [
            (TabKind::Telemetry, "遥测"),
            (TabKind::Params, "参数"),
            (TabKind::Mission, "航点"),
            (TabKind::Map, "地图"),
            (TabKind::Log, "日志"),
        ] {
            if ui.button(label).clicked() {
                state.tab = tk;
                if tk == TabKind::Params {
                    app.request_params();
                }
            }
        }
    });
}
