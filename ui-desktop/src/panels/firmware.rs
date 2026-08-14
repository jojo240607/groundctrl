//! 固件升级面板：通过 MAVLink FTP（FILE_TRANSFER_PROTOCOL）上传/下载/校验固件（P3-1）。

use egui::{Color32, RichText, Ui};

use crate::app::GroundControlApp;
use crate::state::UiState;

pub fn firmware_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    let lang = state.lang;
    ui.heading(lang.tr("固件升级（MAVLink FTP）"));
    ui.label(
        RichText::new(
            "通过 MAVLink FTP 协议向飞控上传固件镜像（FILE_TRANSFER_PROTOCOL, msg 110）。\
             上传采用 CreateFile + 分块 WriteFile + TerminateSession 流程，每块等待飞控 ACK 确认。",
        )
        .color(Color32::GRAY),
    );

    ui.separator();

    // 目标机：当前选中飞机（未连接时默认 1/1）
    let (sys, comp) = if state.vehicle.online {
        (state.vehicle.sys_id, state.vehicle.comp_id)
    } else {
        (1, 1)
    };
    ui.horizontal(|ui| {
        ui.label(format!("目标: 系统 {sys} / 组件 {comp}"));
    });

    // ---- 1) 选择本地固件文件 ----
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!state.fw_busy, egui::Button::new(lang.tr("选择固件文件...")))
            .clicked()
        {
            pick_firmware(state, app);
        }
        if let Some((name, data)) = &state.fw_file {
            ui.label(format!("{name}  ({:.1} KB)", data.len() as f64 / 1024.0));
        } else {
            ui.label(RichText::new(lang.tr("未选择文件")).color(Color32::GRAY));
        }
    });

    // ---- 2) 上传 ----
    let can_upload = state.fw_file.is_some() && !state.fw_busy;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(can_upload, egui::Button::new(lang.tr("上传到飞控")))
            .clicked()
        {
            upload_firmware(state, app, sys, comp);
        }
        if state.fw_busy {
            ui.spinner();
        }
        if state.fw_progress > 0.0 && state.fw_progress < 1.0 {
            ui.add(
                egui::ProgressBar::new(state.fw_progress)
                    .show_percentage()
                    .desired_width(220.0),
            );
        }
    });

    // ---- 3) 下载回读校验 / 计算校验和 ----
    let sel_file = state
        .fw_files
        .iter()
        .find(|(_, n)| *n == state.fw_sel_name)
        .map(|(_, n)| n.clone());
    ui.horizontal(|ui| {
        let can_dl = sel_file.is_some() && !state.fw_busy;
        if ui
            .add_enabled(can_dl, egui::Button::new(lang.tr("下载到本地...")))
            .clicked()
        {
            if let Some(name) = &sel_file {
                download_firmware(state, app, sys, comp, name);
            }
        }
        if ui
            .add_enabled(sel_file.is_some() && !state.fw_busy, egui::Button::new(lang.tr("计算 CRC32")))
            .clicked()
        {
            if let Some(name) = &sel_file {
                calc_crc(state, app, sys, comp, name);
            }
        }
        if ui
            .add_enabled(sel_file.is_some() && !state.fw_busy, egui::Button::new(lang.tr("删除文件")))
            .clicked()
        {
            if let Some(name) = &sel_file {
                remove_file(state, app, sys, comp, name);
            }
        }
    });

    // ---- 4) 飞控文件列表 ----
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new(lang.tr("飞控存储")).strong());
        if ui
            .add_enabled(!state.fw_busy, egui::Button::new(lang.tr("刷新列表")))
            .clicked()
        {
            list_files(state, app, sys, comp);
        }
    });
    if state.fw_files.is_empty() {
        ui.label(RichText::new(lang.tr("（空）")).color(Color32::GRAY));
    } else {
        egui::Grid::new("fw_files").striped(true).show(ui, |ui| {
            for (t, name) in &state.fw_files {
                let is_file = *t == groundctrl_core::mlink::ftp::FILETYPE_FILE;
                let label = if is_file { name.clone() } else { format!("[目录] {name}") };
                if ui
                    .selectable_label(state.fw_sel_name == *name, label)
                    .clicked()
                {
                    state.fw_sel_name = name.clone();
                }
                ui.label(if is_file { lang.tr("文件") } else { lang.tr("目录") });
                ui.end_row();
            }
        });
    }

    if !state.fw_msg.is_empty() {
        ui.separator();
        ui.label(RichText::new(&state.fw_msg).color(Color32::LIGHT_BLUE));
    }
}

/// 选择本地固件文件并读入内存
fn pick_firmware(_state: &mut UiState, app: &GroundControlApp) {
    let st = app.state.clone();
    let rt = app.rt.handle().clone();
    rt.spawn(async move {
        let file = rfd::AsyncFileDialog::new()
            .add_filter("固件", &["bin", "apj", "px4", "elf", "fw"])
            .set_title("选择固件文件")
            .pick_file()
            .await;
        let Some(file) = file else { return };
        let data = std::fs::read(file.path());
        let mut s = st.lock().unwrap();
        match data {
            Ok(bytes) => {
                s.fw_file = Some((file.file_name().to_string(), bytes));
                s.fw_msg = "固件文件已载入，可点击上传".into();
            }
            Err(e) => s.fw_msg = format!("读取文件失败: {e}"),
        }
    });
}

/// 上传固件（CreateFile + 分块 WriteFile，进度实时回填）
fn upload_firmware(state: &mut UiState, app: &GroundControlApp, sys: u8, comp: u8) {
    let Some((name, data)) = state.fw_file.clone() else { return };
    state.fw_busy = true;
    state.fw_progress = 0.0;
    state.fw_msg = format!("开始上传 {name}...");
    let hub = app.hub.clone();
    let st = app.state.clone();
    let rt = app.rt.handle().clone();
    rt.spawn(async move {
        let res = hub
            .upload_firmware(sys, comp, &name, &data, |sent, total| {
                if let Ok(mut s) = st.lock() {
                    s.fw_progress = if total > 0 { sent as f32 / total as f32 } else { 0.0 };
                }
            })
            .await;
        if let Ok(mut s) = st.lock() {
            s.fw_busy = false;
            s.fw_msg = match res {
                Ok(_) => format!("✅ {name} 上传完成（{} 字节），可刷新列表查看", data.len()),
                Err(e) => format!("上传失败: {e}"),
            };
            s.fw_progress = 1.0;
        }
    });
}

/// 从飞控下载文件到本地
fn download_firmware(state: &mut UiState, app: &GroundControlApp, sys: u8, comp: u8, name: &str) {
    state.fw_busy = true;
    state.fw_msg = format!("正在下载 {name}...");
    let hub = app.hub.clone();
    let st = app.state.clone();
    let rt = app.rt.handle().clone();
    let name = name.to_string();
    rt.spawn(async move {
        let res = hub.download_firmware(sys, comp, &name).await;
        // 块作用域：确保 MutexGuard 在 await 之前释放
        let bytes = {
            let mut s = st.lock().unwrap();
            s.fw_busy = false;
            match res {
                Ok(b) => b,
                Err(e) => {
                    s.fw_msg = format!("下载失败: {e}");
                    return;
                }
            }
        };
        // 弹出保存对话框（rfd 需要非阻塞上下文）
        let path = rfd::AsyncFileDialog::new()
            .set_file_name(&name)
            .save_file()
            .await;
        let mut s = st.lock().unwrap();
        if let Some(p) = path {
            match std::fs::write(p.path(), &bytes) {
                Ok(_) => s.fw_msg = format!("✅ 已保存 {name}（{} 字节）", bytes.len()),
                Err(e) => s.fw_msg = format!("保存失败: {e}"),
            }
        } else {
            s.fw_msg = "已取消保存".into();
        }
    });
}

/// 计算飞控侧文件 CRC32；若已选中本地文件则同时计算本地 CRC 并对比
fn calc_crc(state: &mut UiState, app: &GroundControlApp, sys: u8, comp: u8, name: &str) {
    state.fw_busy = true;
    state.fw_msg = format!("正在计算 {name} 的 CRC32...");
    let hub = app.hub.clone();
    let st = app.state.clone();
    let rt = app.rt.handle().clone();
    let name = name.to_string();
    let local = state.fw_file.clone().map(|(_, d)| d);
    rt.spawn(async move {
        let res = hub.ftp_file_crc32(sys, comp, &name).await;
        let mut s = st.lock().unwrap();
        s.fw_busy = false;
        match res {
            Ok(remote_crc) => {
                if let Some(local) = &local {
                    let local_crc = groundctrl_core::mlink::ftp::crc32_ieee(local);
                    if local_crc == remote_crc {
                        s.fw_msg = format!("✅ {name} CRC32 = {:08X}，与本地文件一致", remote_crc);
                    } else {
                        s.fw_msg = format!(
                            "⚠️ CRC32 不一致：飞控 {:08X} vs 本地 {:08X}",
                            remote_crc, local_crc
                        );
                    }
                } else {
                    s.fw_msg = format!("{name} CRC32 = {:08X}", remote_crc);
                }
            }
            Err(e) => s.fw_msg = format!("CRC32 计算失败: {e}"),
        }
    });
}

/// 删除飞控侧文件
fn remove_file(state: &mut UiState, app: &GroundControlApp, sys: u8, comp: u8, name: &str) {
    state.fw_busy = true;
    state.fw_msg = format!("正在删除 {name}...");
    let hub = app.hub.clone();
    let st = app.state.clone();
    let rt = app.rt.handle().clone();
    let name = name.to_string();
    rt.spawn(async move {
        let res = hub.ftp_remove_file(sys, comp, &name).await;
        let mut s = st.lock().unwrap();
        s.fw_busy = false;
        match res {
            Ok(_) => {
                s.fw_msg = format!("已删除 {name}");
                s.fw_files.retain(|(_, n)| n != &name);
                if s.fw_sel_name == name {
                    s.fw_sel_name.clear();
                }
            }
            Err(e) => s.fw_msg = format!("删除失败: {e}"),
        }
    });
}

/// 刷新飞控 FTP 文件列表
fn list_files(state: &mut UiState, app: &GroundControlApp, sys: u8, comp: u8) {
    state.fw_busy = true;
    state.fw_msg = "正在读取飞控文件列表...".into();
    let hub = app.hub.clone();
    let st = app.state.clone();
    let rt = app.rt.handle().clone();
    rt.spawn(async move {
        let res = hub.ftp_list_directory(sys, comp).await;
        let mut s = st.lock().unwrap();
        s.fw_busy = false;
        match res {
            Ok(entries) => {
                s.fw_files = entries;
                s.fw_msg = format!("共 {} 个条目", s.fw_files.len());
            }
            Err(e) => s.fw_msg = format!("读取列表失败: {e}"),
        }
    });
}
