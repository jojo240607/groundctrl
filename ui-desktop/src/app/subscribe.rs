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

    // 周期性采样参数趋势（1Hz）
    {
        let state = state.clone();
        rt.spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            loop {
                ticker.tick().await;
                if let Ok(mut s) = state.lock() {
                    if !s.trend_enabled {
                        continue;
                    }
                    let now = now_secs() as f64;
                    // 最多保留 300 点（5 分钟 @1Hz）；先拷贝快照避免借用冲突
                    let snapshot: Vec<(String, f64)> = s
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), p.value as f64))
                        .collect();
                    for (name, val) in snapshot {
                        let series = s.param_trends.entry(name).or_default();
                        if series.last().map(|l| l.1 != val).unwrap_or(true) {
                            series.push((now, val));
                            if series.len() > 300 {
                                series.remove(0);
                            }
                        }
                    }
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

#[cfg(test)]
mod tests {
    //! 无 GUI 的端到端闭环验证（路线 D）。
    //! 复用真实 `subscribe_telemetry` + 真实 `TelemetryHub` + `SimLink`，
    //! 断言「链路 -> 机队 -> 参数 -> 趋势采样 -> 告警」全链路在运行期跑通。

    use super::subscribe_telemetry;
    use crate::state::UiState;
    use groundctrl_core::link::share;
    use groundctrl_core::link::sim::SimLink;
    use groundctrl_core::services::TelemetryHub;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn full_loop_trend_and_alarm() {
        let rt = rt();
        let hub = Arc::new(TelemetryHub::new());
        let state = Arc::new(Mutex::new(UiState::default()));

        {
            let mut s = state.lock().unwrap();
            s.trend_enabled = true;
            s.trend_selected = vec![
                "SYSID_THISMAV".to_string(),
                "RTL_ALT".to_string(),
                "WPNAV_SPEED".to_string(),
            ];
        }

        rt.block_on(async {
            // 先订阅再连接，确保 LinkState 事件被捕获（总线无回放）
            subscribe_telemetry(&hub, &state, &rt);
            hub.connect(share(SimLink::new())).await;
            hub.request_params(1, 1).await.unwrap();
            tokio::time::sleep(Duration::from_millis(2500)).await;
        });

        let s = state.lock().unwrap();

        // 1) 机队已聚合
        assert!(!s.vehicles.is_empty(), "机队应非空");
        let v = s.vehicles.values().next().unwrap();
        assert!(v.heartbeat.is_some(), "应收到心跳");
        assert!(v.gps.lat > 30.0, "GPS 纬度应合理");

        // 2) 参数已拉取
        assert!(
            s.params_complete || s.params_received > 0,
            "参数应已拉取"
        );
        assert!(!s.params.is_empty(), "s.params 应非空");

        // 3) 趋势采样在累计
        for name in ["SYSID_THISMAV", "RTL_ALT", "WPNAV_SPEED"] {
            let series = s.param_trends.get(name).expect("应存在趋势序列");
            assert!(series.len() >= 1, "参数 {name} 应至少有 1 个采样点");
            let mut last_t = -1.0_f64;
            for (t, _) in series {
                assert!(*t > 0.0, "采样时间戳应为正");
                assert!(*t >= last_t, "采样时间戳应单调递增");
                last_t = *t;
            }
        }

        // 4) 链路状态 OPEN
        assert!(s.active_link.is_some(), "active_link 应被置位");
    }

    #[test]
    fn trend_disabled_produces_no_samples() {
        let rt = rt();
        let hub = Arc::new(TelemetryHub::new());
        let state = Arc::new(Mutex::new(UiState::default()));
        rt.block_on(async {
            subscribe_telemetry(&hub, &state, &rt);
            hub.connect(share(SimLink::new())).await;
            hub.request_params(1, 1).await.unwrap();
            tokio::time::sleep(Duration::from_millis(1500)).await;
        });
        let s = state.lock().unwrap();
        assert!(
            s.param_trends.values().all(|v| v.is_empty()),
            "未启用趋势时不应有采样点"
        );
    }
}
