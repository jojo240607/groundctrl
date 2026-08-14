//! 传感器校准向导：通过 MAV_CMD_PREFLIGHT_CALIBRATION 触发飞控校准，
//! 结果经 COMMAND_ACK 异步回传显示（P2-2）。

use egui::{Color32, RichText, Ui};

use crate::app::GroundControlApp;
use crate::state::UiState;

/// 校准项：位掩码 + 名称 + 操作提示
struct CalItem {
    what: u8,
    name: &'static str,
    hint: &'static str,
}

const ITEMS: [CalItem; 6] = [
    CalItem {
        what: 128,
        name: "水平校准",
        hint: "将飞行器放置在水平面上进行",
    },
    CalItem {
        what: 2,
        name: "加速度计",
        hint: "需要 6 面姿态（平放/侧翻/倒置等），跟随飞控提示音操作",
    },
    CalItem {
        what: 4,
        name: "磁罗盘",
        hint: "远离金属与磁场干扰源，原地旋转飞行器",
    },
    CalItem {
        what: 8,
        name: "气压计",
        hint: "静止状态下执行，校准前后勿改变高度",
    },
    CalItem {
        what: 1,
        name: "陀螺仪",
        hint: "保持飞行器静止即可",
    },
    CalItem {
        what: 0,
        name: "全部校准",
        hint: "依次执行陀螺仪/加速度计/磁罗盘/气压计/水平校准（耗时较长）",
    },
];

pub fn calibration_panel(ui: &mut Ui, state: &mut UiState, app: &GroundControlApp) {
    let lang = state.lang;
    ui.heading(lang.tr("传感器校准向导"));
    ui.label(
        RichText::new(
            "校准前请确保飞行器已上锁（未解锁）、电量充足并静止放置。\
             发送 MAV_CMD_PREFLIGHT_CALIBRATION，结果通过 COMMAND_ACK 回传。",
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

    for item in ITEMS {
        ui.horizontal(|ui| {
            ui.label(RichText::new(item.name).strong());
            ui.label(RichText::new(item.hint).color(Color32::GRAY));
            if ui
                .add_enabled(!state.cal_busy, egui::Button::new(lang.tr("开始校准")))
                .clicked()
            {
                let hub = app.hub.clone();
                let rt = app.rt.handle().clone();
                let st = app.state.clone();
                let what = item.what;
                let name = item.name;
                rt.spawn(async move {
                    if let Ok(mut s) = st.lock() {
                        s.cal_busy = true;
                        s.cal_msg = format!("正在发送 {name} 校准指令...");
                    }
                    let res = hub.send_calibrate(sys, comp, what).await;
                    if let Ok(mut s) = st.lock() {
                        s.cal_busy = false;
                        s.cal_msg = match res {
                            Ok(_) => format!("{name} 校准指令已发送，等待回执..."),
                            Err(e) => format!("发送失败: {e}"),
                        };
                    }
                });
            }
        });
        ui.separator();
    }

    if state.cal_busy {
        ui.label(RichText::new(lang.tr("校准进行中...")).color(Color32::from_rgb(255, 140, 0)));
    }
    if !state.cal_msg.is_empty() {
        ui.label(RichText::new(&state.cal_msg).color(Color32::LIGHT_BLUE));
    }

    // 最近指令回执（来自当前选中飞机）
    let v = &state.vehicle;
    if let Some(txt) = v.ack_text() {
        let ok = v.cmd_ack.as_ref().map(|a| a.result == 0).unwrap_or(false);
        let col = if ok { Color32::GREEN } else { Color32::from_rgb(255, 140, 0) };
        ui.label(RichText::new(format!("{} {txt}", lang.tr("回执:"))).color(col));
    }
}
