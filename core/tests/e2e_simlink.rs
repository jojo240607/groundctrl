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
