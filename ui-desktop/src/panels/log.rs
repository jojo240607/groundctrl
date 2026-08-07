//! 日志面板：tlog 保存/加载 + CSV/KML 轨迹导出。

use egui::{RichText, Ui};

use groundctrl_core::services::log::LogManager;

use crate::app::GroundControlApp;
use crate::state::UiState;
use crate::widgets::export::{export_track_csv, export_track_kml};

pub fn log_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("飞行日志 (tlog)");
    ui.label(format!("已记录帧数: {}", state.log_frames));
    ui.label("日志在 TelemetryHub 中实时记录每条遥测帧（tlog 格式）。");

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("保存 tlog...").clicked() {
            save_tlog(state, app);
        }
        if ui.button("加载 tlog...").clicked() {
            load_tlog(state, app);
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("导出轨迹 CSV...").clicked() {
            export_csv(state, app);
        }
        if ui.button("导出轨迹 KML...").clicked() {
            export_kml(state, app);
        }
    });

    if !state.export_msg.is_empty() {
        ui.label(RichText::new(&state.export_msg).color(egui::Color32::GREEN));
    }
    if !state.import_msg.is_empty() {
        ui.label(RichText::new(&state.import_msg).color(egui::Color32::LIGHT_BLUE));
    }
    ui.label("CSV/KML 由当前 GPS 轨迹（与本地航点）生成，可用于 Google Earth / 表格分析。");
}

/// 保存当前 hub 日志为 tlog 文件
fn save_tlog(state: &mut UiState, app: &GroundControlApp) {
    let hub = app.hub.clone();
    let rt = app.rt.handle().clone();
    let st = app.state.clone();
    let _ = state;
    rt.spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_title("保存飞行日志")
            .set_file_name("flight.tlog")
            .save_file()
            .await
        {
            let path = path.path().to_path_buf();
            let log_arc = hub.log();
            let res = {
                let log = log_arc.lock().await;
                log.save_file(path.to_str().unwrap_or("flight.tlog"))
            };
            if let Ok(mut s) = st.lock() {
                s.export_msg = match res {
                    Ok(_) => format!("已保存: {}", path.display()),
                    Err(e) => format!("保存失败: {e}"),
                };
            }
        }
    });
}

/// 从 tlog 文件加载日志并替换 hub 当前日志
fn load_tlog(state: &mut UiState, app: &GroundControlApp) {
    let hub = app.hub.clone();
    let rt = app.rt.handle().clone();
    let st = app.state.clone();
    let _ = state;
    rt.spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_title("加载飞行日志")
            .add_filter("tlog", &["tlog"])
            .pick_file()
            .await
        {
            let path = path.path().to_path_buf();
            let sp = path.to_str().unwrap_or("flight.tlog").to_string();
            let loaded = LogManager::load_file(&sp);
            let mut msg = String::new();
            if let Ok(lm) = &loaded {
                let log_arc = hub.log();
                *log_arc.lock().await = lm.clone();
                msg = format!("已加载 {} 帧: {}", lm.len(), path.display());
            } else if let Err(e) = &loaded {
                msg = format!("加载失败: {e}");
            }
            if let Ok(mut s) = st.lock() {
                s.import_msg = msg;
            }
        }
    });
}

/// 导出 GPS 轨迹为 CSV
fn export_csv(state: &mut UiState, app: &GroundControlApp) {
    let rt = app.rt.handle().clone();
    let st = app.state.clone();
    let _ = state;
    rt.spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_title("导出轨迹 CSV")
            .set_file_name("track.csv")
            .save_file()
            .await
        {
            let path = path.path().to_path_buf();
            let (csv, n) = {
                let s = st.lock().unwrap();
                let tr = s.active_trail();
                (export_track_csv(&tr), tr.len())
            };
            let res = std::fs::write(&path, csv);
            if let Ok(mut s) = st.lock() {
                s.export_msg = match res {
                    Ok(_) => format!("已导出 CSV: {n} 点 ({})", path.display()),
                    Err(e) => format!("导出失败: {e}"),
                };
            }
        }
    });
}

/// 导出 GPS 轨迹 + 航点为 KML
fn export_kml(state: &mut UiState, app: &GroundControlApp) {
    let rt = app.rt.handle().clone();
    let st = app.state.clone();
    let _ = state;
    rt.spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_title("导出轨迹 KML")
            .set_file_name("track.kml")
            .save_file()
            .await
        {
            let path = path.path().to_path_buf();
            let (kml, n) = {
                let s = st.lock().unwrap();
                let tr = s.active_trail();
                (export_track_kml(&tr, &s.mission), tr.len())
            };
            let res = std::fs::write(&path, kml);
            if let Ok(mut s) = st.lock() {
                s.export_msg = match res {
                    Ok(_) => format!("已导出 KML: {n} 点 ({})", path.display()),
                    Err(e) => format!("导出失败: {e}"),
                };
            }
        }
    });
}
