//! 后台订阅任务：把 core 的遥测快照与总线事件转发为 Tauri 事件推送给前端。

use crate::frontend::*;
use groundctrl_core::proto::bus::BusEvent;
use groundctrl_core::services::alarms::AlarmLevel;
use groundctrl_core::services::TelemetryHub;
use tauri::{AppHandle, Emitter};

/// 启动遥测订阅后台任务。该任务：
/// 1. 每 200ms 把机队快照推送为 "fleet" 事件；
/// 2. 订阅 core 总线，把告警 / 链路状态 / 参数值 / 航点 转发为对应事件。
///
/// 返回的任务句柄应在状态中保存，便于停止/重启。
pub fn start_subscription(app: &AppHandle, hub: TelemetryHub) {
    let app = app.clone();

    // 快照周期推送
    let hub_snap = hub.clone();
    let snap_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(200));
        loop {
            tick.tick().await;
            let fleet = build_fleet(&hub_snap).await;
            let _ = snap_app.emit("fleet", &fleet);
        }
    });

    // 总线事件转发
    let hub_bus = hub.clone();
    tauri::async_runtime::spawn(async move {
        let mut rx = hub_bus.bus().subscribe();
        while let Ok(ev) = rx.recv().await {
            forward_bus_event(&app, ev);
        }
    });
}

/// 构建机队快照。
pub async fn build_fleet(hub: &TelemetryHub) -> FleetSnapshot {
    let fleet = hub.fleet().await;
    let mut vehicles: Vec<VehicleSnapshot> = fleet.iter().map(VehicleSnapshot::from_vehicle).collect();
    vehicles.sort_by_key(|v| v.sysid);
    let selected = vehicles.first().map(|v| v.sysid).unwrap_or(0);
    FleetSnapshot { vehicles, selected }
}

/// 把核心总线事件转译为前端事件。
fn forward_bus_event(app: &AppHandle, ev: BusEvent) {
    match ev {
        BusEvent::Alarm { link, alarm } => {
            let level = match alarm.level {
                AlarmLevel::Info => AlarmLevelJson::Info,
                AlarmLevel::Warn => AlarmLevelJson::Warn,
                AlarmLevel::Critical => AlarmLevelJson::Critical,
            };
            let aj = AlarmJson {
                link: link.clone(),
                code: alarm.code.to_string(),
                level,
                message: alarm.message.clone(),
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            };
            let _ = app.emit("alarm", &aj);
        }
        BusEvent::LinkState { link, open } => {
            let lj = LinkStateJson {
                link: link.clone(),
                connected: open,
            };
            let _ = app.emit("link-state", &lj);
        }
        BusEvent::Params {
            link: _,
            complete,
            received,
            expected,
            entries,
        } => {
            // 推送参数拉取进度
            let _ = app.emit(
                "params-progress",
                serde_json::json!({
                    "complete": complete,
                    "received": received,
                    "expected": expected,
                }),
            );
            // 推送每个参数条目
            for e in entries {
                let item = ParamItem {
                    index: e.index,
                    name: e.name.clone(),
                    value: e.value,
                };
                let _ = app.emit("param-value", &item);
            }
        }
        _ => {}
    }
}
