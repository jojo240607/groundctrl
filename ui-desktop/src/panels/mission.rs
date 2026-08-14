//! 航点面板：本地航点列表 + 新增/删除/排序 + 上传 / 清空。

use egui::{DragValue, RichText, Ui};

use groundctrl_core::vehicle::mission::Waypoint;

use crate::app::GroundControlApp;
use crate::state::UiState;

pub fn mission_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.heading("航点规划");

    ui.horizontal(|ui| {
        ui.label("lat:");
        ui.add(DragValue::new(&mut state.wp_lat).speed(0.0001));
        ui.label("lon:");
        ui.add(DragValue::new(&mut state.wp_lon).speed(0.0001));
        ui.label("alt:");
        ui.add(DragValue::new(&mut state.wp_alt).speed(1.0));
        if ui.button("添加航点").clicked() {
            state
                .mission
                .push(Waypoint::nav(state.wp_lat, state.wp_lon, state.wp_alt));
        }
    });

    // 点击地图添加航点时使用的默认高度，可在此调整
    ui.horizontal(|ui| {
        if ui.button("上传到飞控").clicked() {
            app.upload_mission();
        }
        if ui.button("清空").clicked() {
            state.mission.clear();
        }
        ui.label(format!("共 {} 个航点", state.mission.len()));
        if ui.button("在地图上点选添加").clicked() {
            state.map_click_add_wp = !state.map_click_add_wp;
        }
    });

    // 任务文件：保存 / 加载（.plan JSON）
    ui.horizontal(|ui| {
        if ui.button("保存任务...").clicked() {
            save_plan(state, app);
        }
        if ui.button("加载任务...").clicked() {
            load_plan(state, app);
        }
        if !state.import_msg.is_empty() {
            ui.label(RichText::new(&state.import_msg).color(egui::Color32::LIGHT_BLUE));
        }
    });

    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut to_remove: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;

        for i in 0..state.mission.len() {
            ui.horizontal(|ui| {
                ui.label(format!("#{}", i));
                // 可编辑高度
                ui.add(DragValue::new(&mut state.mission[i].alt).speed(0.5).prefix("alt=").suffix("m"));
                let wp = &state.mission[i];
                ui.label(format!("lat={:.6} lon={:.6}", wp.lat, wp.lon));
                if ui.button("↑").clicked() {
                    if i > 0 {
                        move_up = Some(i);
                    }
                }
                if ui.button("↓").clicked() {
                    if i + 1 < state.mission.len() {
                        move_down = Some(i);
                    }
                }
                if ui.button("删除").clicked() {
                    to_remove = Some(i);
                }
            });
        }

        if let Some(i) = to_remove {
            state.mission.remove(i);
        }
        if let Some(i) = move_up {
            state.mission.swap(i, i - 1);
        }
        if let Some(i) = move_down {
            state.mission.swap(i, i + 1);
        }

        if state.mission.is_empty() {
            ui.label("尚无航点。在上方输入坐标添加，或勾选「在地图上点选添加」后点击地图。");
        }
    });
}

/// 保存本地航点列表为 .plan JSON 文件
fn save_plan(state: &mut UiState, app: &GroundControlApp) {
    let rt = app.rt.handle().clone();
    let st = app.state.clone();
    let _ = state;
    rt.spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_title("保存任务")
            .set_file_name("mission.plan")
            .add_filter("Plan", &["plan", "json"])
            .save_file()
            .await
        {
            let path = path.path().to_path_buf();
            let json = {
                let s = st.lock().unwrap();
                serde_json::to_string_pretty(&s.mission)
            };
            let res = match json {
                Ok(j) => std::fs::write(&path, j),
                Err(e) => {
                    if let Ok(mut s) = st.lock() {
                        s.import_msg = format!("序列化失败: {e}");
                    }
                    return;
                }
            };
            if let Ok(mut s) = st.lock() {
                s.import_msg = match res {
                    Ok(_) => format!("任务已保存: {}", path.display()),
                    Err(e) => format!("保存失败: {e}"),
                };
            }
        }
    });
}

/// 从 .plan JSON 文件加载航点列表
fn load_plan(state: &mut UiState, app: &GroundControlApp) {
    let rt = app.rt.handle().clone();
    let st = app.state.clone();
    let _ = state;
    rt.spawn(async move {
        if let Some(path) = rfd::AsyncFileDialog::new()
            .set_title("加载任务")
            .add_filter("Plan", &["plan", "json"])
            .pick_file()
            .await
        {
            let path = path.path().to_path_buf();
            let loaded = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| {
                    serde_json::from_str::<Vec<Waypoint>>(&s).map_err(|e| e.to_string())
                });
            let msg = match loaded {
                Ok(wps) => {
                    if let Ok(mut s) = st.lock() {
                        s.mission = wps;
                    }
                    format!("已加载任务: {}", path.display())
                }
                Err(e) => format!("加载失败: {e}"),
            };
            if let Ok(mut s) = st.lock() {
                s.import_msg = msg;
            }
        }
    });
}

pub fn fence_section(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    ui.separator();
    ui.heading("地理围栏");

    // FENCE_STATUS 状态显示
    let v = &state.vehicle;
    if v.fence.seen {
        ui.label(format!(
            "围栏状态：{}（违例类型：{}）",
            v.fence.status_text(),
            v.fence.breach_type_name()
        ));
    } else {
        ui.label("围栏状态：未上报（SimLink 周期发送 FENCE_STATUS）");
    }

    ui.separator();

    // 添加点
    ui.horizontal(|ui| {
        ui.label("lat:");
        ui.add(DragValue::new(&mut state.fence_lat).speed(0.0001));
        ui.label("lon:");
        ui.add(DragValue::new(&mut state.fence_lon).speed(0.0001));
        if ui.button("添加围栏点").clicked() {
            state.fence.push((state.fence_lat, state.fence_lon));
        }
    });

    // 操作按钮
    ui.horizontal(|ui| {
        if state.fence.len() >= 3 {
            if ui.button("上传围栏").clicked() {
                app.upload_fence();
            }
        } else {
            ui.label("至少 3 个点才能上传");
        }
        if ui.button("下载围栏").clicked() {
            app.download_fence();
        }
        if ui.button("清空").clicked() {
            state.fence.clear();
        }
        ui.label(format!("{} 个点", state.fence.len()));
    });

    if !state.fence_msg.is_empty() {
        ui.label(RichText::new(&state.fence_msg).color(egui::Color32::LIGHT_BLUE));
    }

    // 围栏点列表
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut to_remove: Option<usize> = None;
        for i in 0..state.fence.len() {
            ui.horizontal(|ui| {
                ui.label(format!("#{}  lat={:.6} lon={:.6}", i, state.fence[i].0, state.fence[i].1));
                if ui.button("删除").clicked() {
                    to_remove = Some(i);
                }
            });
        }
        if let Some(i) = to_remove {
            state.fence.remove(i);
        }
        if state.fence.is_empty() {
            ui.label("尚无围栏点。添加 3 个以上后上传。");
        }
    });
}
