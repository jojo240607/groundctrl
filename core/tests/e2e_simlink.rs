//! 端到端验证：SimLink -> TelemetryHub -> VehicleModel
//!
//! 等价计划 1.6「用 SimLink 注入模拟消息，UI 正确显示」的链路层验证：
//! 不依赖 GUI，断言解析 / 聚合 / 机队更新全链路正确。

use groundctrl_core::link::sim::SimLink;
use groundctrl_core::link::share;
use groundctrl_core::mlink::{self, MavMessage};
use groundctrl_core::proto::bus::BusEvent;
use groundctrl_core::services::TelemetryHub;

#[tokio::test]
async fn encode_parse_roundtrip() {
    // 先单独验证编码 + 解析这一环（SimLink 内部用的同一组 API）
    let msg = MavMessage::HEARTBEAT(::mavlink::common::HEARTBEAT_DATA {
        custom_mode: 0,
        mavtype: ::mavlink::common::MavType::MAV_TYPE_QUADROTOR,
        autopilot: ::mavlink::common::MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
        base_mode: ::mavlink::common::MavModeFlag::MAV_MODE_FLAG_CUSTOM_MODE_ENABLED,
        system_status: ::mavlink::common::MavState::MAV_STATE_ACTIVE,
        mavlink_version: 3,
    });
    let bytes = mlink::encode_v2(&mlink::default_header(), &msg).expect("encode must succeed");
    assert!(!bytes.is_empty(), "编码结果不应为空");

    let mut parser = mlink::MavlinkParser::new();
    let frames = parser.feed(&bytes).expect("parse must succeed");
    assert_eq!(frames.len(), 1, "应解析出一帧");
    assert!(matches!(frames[0].1, MavMessage::HEARTBEAT(_)));
}

#[tokio::test]
async fn simlink_feeds_vehicle_model() {
    let hub = TelemetryHub::new();
    let link = share(SimLink::new());
    hub.attach(link);

    // SimLink 每 100ms 推送一帧（心跳/姿态/电量/定位），等 1.2s 足够多帧
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    let fleet = hub.fleet().await;
    assert!(!fleet.is_empty(), "机队应为非空：SimLink 已注入遥测");

    let v = &fleet[0];
    assert!(v.heartbeat.is_some(), "应收到心跳");
    assert!(
        v.attitude.roll.abs() + v.attitude.pitch.abs() + v.attitude.yaw.abs() > 0.0,
        "姿态应随合成数据缓慢变化（非全零）"
    );
    // GPS 经纬度为 deg 单位，合成纬度约 31.1°
    assert!(v.gps.lat > 30.0, "纬度应在合理范围");
    assert!(v.battery.voltage > 0.0, "应收到电池电压");

    // 通过总线也能收到 Mavlink 事件
    let mut rx = hub.bus().subscribe();
    // 再等一帧，确认总线持续产出
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let mut got = false;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, BusEvent::Mavlink { .. }) {
            got = true;
        }
    }
    assert!(got, "总线应持续产出 Mavlink 事件");
}

#[tokio::test]
async fn connect_replaces_active_link() {
    use groundctrl_core::link::sim::SimLink;
    use groundctrl_core::link::share;

    let hub = TelemetryHub::new();

    // 第一次连接 Sim
    hub.connect(share(SimLink::new())).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(hub.current_link_name().await.is_some(), "应有活动链路");

    // 第二次连接应替换（不抛错、仍有一条活动链路）
    hub.connect(share(SimLink::new())).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(hub.current_link_name().await.is_some(), "替换后仍应有活动链路");
    let fleet = hub.fleet().await;
    assert!(!fleet.is_empty(), "替换链路后机队不应被清空");

    // 断开
    hub.disconnect().await;
    assert!(hub.current_link_name().await.is_none(), "断开后无活动链路");
    // 断开不应清除已聚合的遥测
    assert!(!hub.fleet().await.is_empty(), "断开后已聚合数据应保留");
}

#[tokio::test]
async fn param_pull_over_simlink() {
    use groundctrl_core::link::sim::SimLink;
    use groundctrl_core::link::share;
    use groundctrl_core::proto::bus::BusEvent;

    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    // 订阅参数事件
    let mut rx = hub.bus().subscribe();
    // 请求参数（飞控 sys=1, comp=1）
    hub.request_params(1, 1).await.unwrap();
    // 等 SimLink 回放 PARAM_VALUE
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let mut got_params = false;
    let mut entries = 0;
    let mut complete = false;
    while let Ok(ev) = rx.try_recv() {
        if let BusEvent::Params {
            complete: c,
            received,
            ..
        } = ev
        {
            got_params = true;
            entries = received as usize;
            complete = c; // 取最后一次（received 最大）的状态
        }
    }
    assert!(got_params, "总线应产出 Params 事件");
    assert!(complete, "SimLink 参数应集满");
    assert!(entries >= 5, "应至少拉到 5 个示例参数");
}

#[tokio::test]
async fn alarm_triggers_on_low_battery() {
    use groundctrl_core::link::sim::SimLink;
    use groundctrl_core::link::share;
    use groundctrl_core::proto::bus::BusEvent;
    use groundctrl_core::services::alarms::AlarmLevel;

    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    let mut rx = hub.bus().subscribe();
    // 电量每 2s 降 1%，从 80 降到 <=15 需要 ~130s，太久；
    // 这里直接验证监控器逻辑（单元级），链路级只确认无告警时安静。
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let mut saw_critical = false;
    while let Ok(ev) = rx.try_recv() {
        if let BusEvent::Alarm { alarm, .. } = ev {
            // 初期电量 80%（Some(80)），不应有低电量严重告警
            if alarm.level == AlarmLevel::Critical && alarm.code == "BATT_CRIT" {
                saw_critical = true;
            }
        }
    }
    assert!(!saw_critical, "高电量（80%）不应触发严重低电量告警");
}

#[tokio::test]
async fn logging_records_frames() {
    use groundctrl_core::link::sim::SimLink;
    use groundctrl_core::link::share;

    let hub = TelemetryHub::new();
    hub.set_logging(true).await;
    hub.connect(share(SimLink::new())).await;

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let n = hub.log_frames().await;
    assert!(n > 0, "日志应记录到多帧遥测");

    // 导出 tlog 并回放
    let log = hub.log();
    let mgr = log.lock().await;
    let tlog = mgr.to_tlog();
    assert!(!tlog.is_empty(), "tlog 不应为空");

    let replayed = groundctrl_core::services::log::LogManager::from_tlog(&tlog)
        .expect("tlog 应可回放");
    assert_eq!(replayed.len(), n, "回放帧数应一致");
}

#[test]
fn flight_monitor_unit() {
    use groundctrl_core::services::alarms::{AlarmLevel, FlightMonitor, MonitorConfig};
    use groundctrl_core::vehicle::VehicleModel;

    // 默认配置：低电量阈值 15/30
    let mut mon = FlightMonitor::with_defaults();
    let mut v = VehicleModel::default();
    v.battery.remaining_pct = Some(10); // 极低
    let alarms = mon.evaluate(&v);
    assert!(
        alarms.iter().any(|a| a.level == AlarmLevel::Critical && a.code == "BATT_CRIT"),
        "电量 10% 应触发严重低电量告警"
    );

    // 围栏越界
    let mut mon2 = FlightMonitor::new(MonitorConfig {
        fence_lat: 31.0,
        fence_lon: 121.0,
        fence_radius_m: 1000.0,
        ..MonitorConfig::default()
    });
    let mut v2 = VehicleModel::default();
    v2.gps.lat = 31.05; // 约 5.5km 外
    v2.gps.lon = 121.05;
    let alarms2 = mon2.evaluate(&v2);
    assert!(
        alarms2.iter().any(|a| a.code == "FENCE"),
        "远离围栏中心应触发越界告警"
    );
}

