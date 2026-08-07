//! 启动各类后台订阅任务：遥测快照、总线事件、日志帧数刷新。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use groundctrl_core::proto::bus::BusEvent;
use groundctrl_core::services::TelemetryHub;

use crate::app::now_secs;
use crate::state::UiState;

/// 启动所有后台订阅任务（在 app 构造时调用一次）
pub fn subscribe_telemetry(
    hub: &Arc<TelemetryHub>,
    state: &Arc<Mutex<UiState>>,
    rt: &tokio::runtime::Runtime,
) {
    // 订阅遥测，写入共享状态
    {
        let hub = hub.clone();
        let state = state.clone();
        rt.spawn(async move {
            let mut rx = hub.subscribe_telemetry();
            loop {
                match rx.recv().await {
                    Ok(vm) => {
                        if let Ok(mut s) = state.lock() {
                            let sys = vm.sys_id;
                            // 记录每架飞机的 GPS 轨迹
                            let traj = s.trails.entry(sys).or_default();
                            if vm.gps.lat != 0.0 || vm.gps.lon != 0.0 {
                                if traj.last() != Some(&(vm.gps.lat, vm.gps.lon)) {
                                    traj.push((vm.gps.lat, vm.gps.lon));
                                    if traj.len() > 2000 {
                                        traj.remove(0);
                                    }
                                }
                            }
                            // 写入机队字典
                            s.vehicles.insert(sys, vm.clone());
                            // 选中飞机：未指定则自动选最新一帧的系统
                            let sel = s.selected_sys.unwrap_or(sys);
                            if sel == sys {
                                s.vehicle = vm.clone();
                            } else if let Some(v) = s.vehicles.get(&sel) {
                                s.vehicle = v.clone();
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // 订阅总线事件（链路状态 / 参数 / 告警）
    {
        let hub = hub.clone();
        let state = state.clone();
        rt.spawn(async move {
            let mut rx = hub.bus().subscribe();
            loop {
                match rx.recv().await {
                    Ok(ev) => match ev {
                        BusEvent::LinkState { link, open } => {
                            if let Ok(mut s) = state.lock() {
                                if open {
                                    s.active_link = Some(link);
                                } else if s.active_link.as_deref() == Some(link.as_str()) {
                                    s.active_link = None;
                                }
                                s.link_status = match &s.active_link {
                                    Some(n) => format!("{n}: OPEN"),
                                    None => "无连接".to_string(),
                                };
                            }
                        }
                        BusEvent::Params {
                            complete,
                            received,
                            expected,
                            entries,
                            ..
                        } => {
                            if let Ok(mut s) = state.lock() {
                                s.params = entries;
                                s.params_complete = complete;
                                s.params_received = received;
                                s.params_expected = expected;
                            }
                        }
                        BusEvent::Alarm { alarm, .. } => {
                            if let Ok(mut s) = state.lock() {
                                s.alarms.push((now_secs(), alarm));
                                if s.alarms.len() > 50 {
                                    s.alarms.remove(0);
                                }
                            }
                        }
                        _ => {}
                    },
                    Err(_) => break,
                }
            }
        });
    }

    // 周期性刷新日志帧数
    {
        let hub = hub.clone();
        let state = state.clone();
        rt.spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(500));
            loop {
                ticker.tick().await;
                let n = hub.log_frames().await;
                if let Ok(mut s) = state.lock() {
                    s.log_frames = n;
                }
            }
        });
    }
}
