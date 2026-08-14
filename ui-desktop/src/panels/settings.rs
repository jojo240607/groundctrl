//! 设置面板：编辑连接默认值、地图/瓦片源、窗口偏好，并落盘到本地 JSON。

use egui::{Slider, Ui};

use crate::app::{GroundControlApp, TabKind};
use crate::state::UiState;

pub fn settings_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    let lang = state.lang;
    ui.heading(lang.tr("设置"));
    ui.label("修改后点击「保存到磁盘」。窗口尺寸会自动记录。");

    ui.separator();
    // 语言切换（P3-3）
    ui.horizontal(|ui| {
        ui.label(lang.tr("语言"));
        let mut sel = state.lang;
        egui::ComboBox::from_id_source("lang_combo")
            .selected_text(sel.label())
            .show_ui(ui, |ui| {
                for l in [crate::i18n::Lang::ZhCn, crate::i18n::Lang::EnUs] {
                    ui.selectable_value(&mut sel, l, l.label());
                }
            });
        if sel != state.lang {
            state.lang = sel;
            app.save_settings();
        }
    });

    ui.separator();
    ui.collapsing(lang.tr("连接默认值"), |ui| {
        ui.horizontal(|ui| {
            ui.label("串口:");
            ui.text_edit_singleline(&mut state.serial_port);
        });
        ui.add(Slider::new(&mut state.baud, 9600..=921600).logarithmic(true).text(lang.tr("波特率")));
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
    ui.collapsing(lang.tr("地图 / 瓦片"), |ui| {
        ui.checkbox(&mut state.online_tiles, lang.tr("启用在线瓦片下载 (开箱即用)"));
        ui.horizontal(|ui| {
            ui.label(lang.tr("瓦片源 URL:"));
            ui.text_edit_singleline(&mut state.tile_url);
        });
        ui.add(Slider::new(&mut state.map_zoom, 2.0..=18.0).logarithmic(true).text(lang.tr("默认缩放")));
        ui.horizontal(|ui| {
            if let Some(d) = &state.tile_dir {
                ui.label(format!("离线瓦片目录: {}", d.display()));
            } else {
                ui.label("瓦片目录: (使用默认在线缓存目录)");
            }
            if ui.button(lang.tr("清除离线目录")).clicked() {
                state.tile_dir = None;
                state.tile_cache.clear();
            }
            if ui.button(lang.tr("选择目录...")).clicked() {
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
        if ui.button(lang.tr("保存到磁盘")).clicked() {
            app.save_settings();
            state.export_msg = "设置已保存".to_string();
        }
        if ui.button(lang.tr("重置为默认")).clicked() {
            *state = UiState::default();
            app.save_settings();
            state.export_msg = "已重置为默认设置并保存".to_string();
        }
    });

    if !state.export_msg.is_empty() {
        ui.label(&state.export_msg);
    }

    ui.separator();
    ui.collapsing(lang.tr("声音告警"), |ui| {
        ui.checkbox(&mut state.sound_enabled, "启用告警提示音（低电量 / 失联 / 越界时蜂鸣）");
        ui.label("提示音按等级区分：严重=急促双音，警告=中频单音，信息=低频短音。");
    });

    ui.separator();
    ui.collapsing(lang.tr("告警规则（电量 / 地理围栏）"), |ui| {
        let mut cfg = state.monitor_cfg.clone();
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("电量预警 % (<)");
            changed |= ui
                .add(egui::DragValue::new(&mut cfg.battery_warn_pct).clamp_range(5..=95))
                .changed();
            ui.label("电量严重 % (<)");
            changed |= ui
                .add(egui::DragValue::new(&mut cfg.battery_critical_pct).clamp_range(1..=90))
                .changed();
        });

        ui.horizontal(|ui| {
            ui.label("围栏半径 (m)");
            changed |= ui
                .add(egui::DragValue::new(&mut cfg.fence_radius_m).clamp_range(10.0..=100000.0))
                .changed();
        });

        ui.horizontal(|ui| {
            ui.label("围栏中心 纬度");
            changed |= ui
                .add(egui::DragValue::new(&mut cfg.fence_lat).speed(0.0001))
                .changed();
            ui.label("经度");
            changed |= ui
                .add(egui::DragValue::new(&mut cfg.fence_lon).speed(0.0001))
                .changed();
        });

        ui.horizontal(|ui| {
            if ui.button("恢复默认阈值").clicked() {
                cfg = groundctrl_core::services::alarms::MonitorConfig::default();
                changed = true;
            }
            if changed {
                ui.label(
                    egui::RichText::new("● 已修改，点「应用告警规则」生效")
                        .color(egui::Color32::YELLOW),
                );
            }
        });

        if ui.button("应用告警规则").clicked() {
            state.monitor_cfg = cfg.clone();
            let rt = app.rt.handle().clone();
            rt.spawn({
                let hub = app.hub.clone();
                async move {
                    hub.set_monitor_config(cfg).await;
                }
            });
        }
    });

    ui.separator();
    ui.label(format!("{} {:?}", lang.tr("当前标签页:"), state.tab));
    // 提供快速跳转到其他面板
    ui.horizontal_wrapped(|ui| {
        for (tk, label) in [
            (TabKind::Telemetry, "遥测"),
            (TabKind::Params, "参数"),
            (TabKind::Mission, "航点"),
            (TabKind::Map, "地图"),
            (TabKind::Log, "日志"),
        ] {
            if ui.button(lang.tr(label)).clicked() {
                state.tab = tk;
                if tk == TabKind::Params {
                    app.request_params();
                }
            }
        }
    });
}
