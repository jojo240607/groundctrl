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
    let msg = MavMessage::HEARTBEAT(mavlink_core::common::HEARTBEAT_DATA {
        custom_mode: 0,
        mavtype: mavlink_core::common::MavType::MAV_TYPE_QUADROTOR,
        autopilot: mavlink_core::common::MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
        base_mode: mavlink_core::common::MavModeFlag::MAV_MODE_FLAG_CUSTOM_MODE_ENABLED,
        system_status: mavlink_core::common::MavState::MAV_STATE_ACTIVE,
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

#[tokio::test]
async fn command_ack_roundtrip_over_simlink() {
    // 飞行指令链路：GCS COMMAND_LONG -> SimLink 模拟飞控 -> COMMAND_ACK -> VehicleModel
    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    // 发送解锁指令（MAV_CMD_COMPONENT_ARM_DISARM = 400）
    hub.send_arm_disarm(1, 1, true).await.unwrap();

    // 等 ACK 回传并聚合到 VehicleModel
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let fleet = hub.fleet().await;
    assert!(!fleet.is_empty(), "机队应为非空");
    let v = &fleet[0];
    let ack = v.cmd_ack.as_ref().expect("应收到 COMMAND_ACK");
    assert_eq!(ack.command, 400, "回执指令应为 ARM/DISARM");
    assert_eq!(ack.result, 0, "SimLink 应回 MAV_RESULT_ACCEPTED");
}

#[tokio::test]
async fn rc_channels_feed_vehicle() {
    // RC_CHANNELS 合成流 -> VehicleModel.rc
    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    let fleet = hub.fleet().await;
    assert!(!fleet.is_empty(), "机队应为非空");
    let v = &fleet[0];
    assert!(v.rc.seen, "应收到 RC_CHANNELS 帧");
    assert_eq!(v.rc.chancount, 8, "合成 8 通道");
    // 通道 3（油门）应持续在有效 PWM 范围（1000..2000）且变化
    assert!(v.rc.ch[2] > 1000 && v.rc.ch[2] < 2000, "油门通道应有效");
    assert!(v.rc.ch[0] > 1000 && v.rc.ch[0] < 2000, "横滚通道应有效");
    assert!(v.rc.link_ok(), "合成 rssi=190 应判定在线");
}

#[tokio::test]
async fn fence_upload_download_roundtrip_over_simlink() {
    // 围栏链路：GCS 上传 FENCE_POINT -> SimLink 模拟飞控存储 -> 下载回读一致
    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    // 上传 4 个点的围栏（多边形）
    let pts = vec![
        (31.2000000, 121.4000000),
        (31.2100000, 121.4100000),
        (31.2200000, 121.3900000),
        (31.2050000, 121.3950000),
    ];
    hub.upload_fence(1, 1, &pts).await.expect("上传应成功");

    // 下载回读
    let got = hub.download_fence(1, 1).await.expect("下载应成功");
    assert_eq!(got.len(), pts.len(), "下载点数应与上传一致");
    for (a, b) in got.iter().zip(pts.iter()) {
        assert!(
            (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6,
            "点应一致: got {a:?} vs exp {b:?}"
        );
    }

    // 围栏状态已聚合进 VehicleModel（SimLink 周期上报 FENCE_STATUS）
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let fleet = hub.fleet().await;
    assert!(!fleet.is_empty(), "机队应为非空");
    assert!(fleet[0].fence.seen, "应收到 FENCE_STATUS 帧");
    assert!(fleet[0].fence.enabled(), "收到状态帧即视为围栏生效");
}

#[tokio::test]
async fn fence_rejects_less_than_three_points() {
    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;
    let err = hub.upload_fence(1, 1, &[(31.0, 121.0), (31.1, 121.1)]).await;
    assert!(err.is_err(), "少于 3 个点应报错");
}

#[test]
fn log_analyze_extracts_series() {
    use groundctrl_core::services::log::LogManager;
    use mavlink_core::common::{
        ATTITUDE_DATA, GLOBAL_POSITION_INT_DATA, GPS_RAW_INT_DATA, SYS_STATUS_DATA, VFR_HUD_DATA,
    };

    let mut lm = LogManager::new();
    lm.set_recording(true);
    let hdr = mlink::MavHeader {
        system_id: 1,
        component_id: 1,
        sequence: 0,
    };
    // 3 帧混合消息，覆盖分析器各分支
    lm.record_msg(
        1000,
        &hdr,
        &MavMessage::GLOBAL_POSITION_INT(GLOBAL_POSITION_INT_DATA {
            time_boot_ms: 1000,
            lat: 0,
            lon: 0,
            alt: 123000,
            relative_alt: 50000,
            vx: 0,
            vy: 0,
            vz: 0,
            hdg: 0,
        }),
    );
    lm.record_msg(
        2000,
        &hdr,
        &MavMessage::VFR_HUD(VFR_HUD_DATA {
            airspeed: 12.5,
            groundspeed: 10.0,
            heading: 90,
            throttle: 60,
            alt: 55.0,
            climb: 1.5,
        }),
    );
    lm.record_msg(
        3000,
        &hdr,
        &MavMessage::SYS_STATUS(SYS_STATUS_DATA {
            onboard_control_sensors_present: mavlink_core::common::MavSysStatusSensor::empty(),
            onboard_control_sensors_enabled: mavlink_core::common::MavSysStatusSensor::empty(),
            onboard_control_sensors_health: mavlink_core::common::MavSysStatusSensor::empty(),
            load: 0,
            voltage_battery: 12400,
            current_battery: 0,
            battery_remaining: 85,
            drop_rate_comm: 0,
            errors_comm: 0,
            errors_count1: 0,
            errors_count2: 0,
            errors_count3: 0,
            errors_count4: 0,
        }),
    );
    lm.record_msg(
        4000,
        &hdr,
        &MavMessage::ATTITUDE(ATTITUDE_DATA {
            time_boot_ms: 4000,
            roll: 0.1,
            pitch: -0.2,
            yaw: 1.0,
            rollspeed: 0.0,
            pitchspeed: 0.0,
            yawspeed: 0.0,
        }),
    );
    lm.record_msg(
        5000,
        &hdr,
        &MavMessage::GPS_RAW_INT(GPS_RAW_INT_DATA {
            time_usec: 5000000,
            fix_type: mavlink_core::common::GpsFixType::GPS_FIX_TYPE_3D_FIX,
            lat: 0,
            lon: 0,
            alt: 0,
            eph: 120,
            epv: 200,
            vel: 1000,
            cog: 0,
            satellites_visible: 11,
            alt_ellipsoid: 0,
            h_acc: 1000,
            v_acc: 2000,
            vel_acc: 500,
            hdg_acc: 100,
            yaw: 0,
        }),
    );
    lm.set_recording(false);

    let series = lm.analyze();
    let get = |name: &str| -> Vec<(f64, f64)> {
        series
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("缺少序列: {name}"))
            .points
            .clone()
    };

    // 相对高度: 50m @ t=0s
    let rel = get("相对高度");
    assert_eq!(rel, vec![(0.0, 50.0)]);
    // 地速 10 m/s @ t=1s，油门 60%
    let gs = get("地速");
    assert_eq!(gs, vec![(1.0, 10.0)]);
    let thr = get("油门");
    assert_eq!(thr, vec![(1.0, 60.0)]);
    // 电量 85% @ t=2s，电压 12.4V
    let batt = get("电量");
    assert_eq!(batt, vec![(2.0, 85.0)]);
    let volt = get("电压");
    assert!((volt[0].1 - 12.4).abs() < 1e-9);
    // 姿态: 弧度转角度
    let roll = get("横滚");
    assert!((roll[0].1 - 0.1 * 180.0 / std::f64::consts::PI).abs() < 1e-6);
    // GPS: HDOP 1.2m @ t=4s，卫星 11
    let hdop = get("HDOP");
    assert!((hdop[0].1 - 1.2).abs() < 1e-9);
    let sats = get("卫星数");
    assert_eq!(sats, vec![(4.0, 11.0)]);
    // 时间轴相对首帧
    assert_eq!(rel[0].0, 0.0);
    assert_eq!(sats[0].0, 4.0);
}

#[tokio::test]
async fn rc_override_echoes_through_simlink() {
    // GCS 下发 RC 覆盖 -> SimLink 模拟飞控应用 -> 周期 RC_CHANNELS 回显覆盖值
    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    // 1) 未覆盖时通道 1 为合成值（随时间摆动，通常非 1200）
    let chans = [1200, 1800, 1600, 1100, 0, 0, 0, 0];
    hub.send_rc_override(1, 1, chans).await.expect("覆盖应发送成功");

    // 2) 等待周期广播（100ms）反映覆盖值
    let mut seen = false;
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let fleet = hub.fleet().await;
        if let Some(v) = fleet.first() {
            if v.rc.seen {
                if v.rc.ch[0] == 1200 && v.rc.ch[1] == 1800 && v.rc.ch[2] == 1600 {
                    seen = true;
                    break;
                }
            }
        }
    }
    assert!(seen, "RC_CHANNELS 应回显 GCS 覆盖值 (chan1=1200 chan2=1800 chan3=1600)");

    // 3) 清除覆盖后不再锁定该值
    hub.clear_rc_override(1, 1).await.expect("清除应成功");
    let mut released = false;
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let fleet = hub.fleet().await;
        if let Some(v) = fleet.first() {
            if v.rc.seen && v.rc.ch[0] != 1200 {
                released = true;
                break;
            }
        }
    }
    assert!(released, "清除覆盖后通道应恢复合成值");
}

/// MAVLink FTP 固件上传 -> 下载回读一致（SimLink 模拟飞控闪存）
#[tokio::test]
async fn firmware_upload_download_roundtrip() {
    use groundctrl_core::mlink::ftp::FILETYPE_FILE;
    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    // 700 字节固件（4 个 WriteFile 块：226*3 + 22）
    let data: Vec<u8> = (0..700u32).map(|i| (i % 251) as u8).collect();
    let name = "firmware.bin";

    // 1) 上传，进度应单调递增至总量
    let mut progress: Vec<(u64, u64)> = Vec::new();
    hub.upload_firmware(1, 1, name, &data, |sent, total| progress.push((sent, total)))
        .await
        .expect("上传应成功");
    assert_eq!(progress.last().map(|(s, t)| (*s, *t)), Some((700, 700)));

    // 2) 下载回读，内容应一致
    let got = hub.download_firmware(1, 1, name).await.expect("下载应成功");
    assert_eq!(got, data, "下载内容应与上传一致");

    // 3) 目录列表应包含该文件
    let entries = hub.ftp_list_directory(1, 1).await.expect("列目录应成功");
    assert!(
        entries.contains(&(FILETYPE_FILE, name.to_string())),
        "目录应包含 {name}: {entries:?}"
    );

    // 4) 重复上传同名文件应被拒绝（FILEEXISTS）
    let err = hub
        .upload_firmware(1, 1, name, &data, |_, _| {})
        .await
        .expect_err("同名文件应拒绝");
    assert!(err.to_string().contains("拒绝"), "错误应说明被拒绝: {err}");
}

/// MAVLink FTP：打开不存在的文件应报 FILENOTFOUND
#[tokio::test]
async fn firmware_download_missing_file() {
    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    let err = hub
        .download_firmware(1, 1, "no_such.bin")
        .await
        .expect_err("不存在文件应报错");
    assert!(err.to_string().contains("拒绝"), "错误应说明被拒绝: {err}");
}

/// 视频（P3-2）：UDP MJPEG 接收，含垃圾前缀 / 跨包重组 / 单包多帧
#[tokio::test]
async fn video_receiver_udp_mjpeg() {
    use groundctrl_core::services::video::VideoReceiver;
    use tokio::net::UdpSocket;

    let rx = VideoReceiver::start("127.0.0.1:0").await.expect("绑定应成功");
    let target = rx.local_addr();

    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sock.connect(target).await.unwrap();

    // 帧模板：真实 JPEG 标记（SOI/EOI）+ 内容字节
    let make = |n: u8| -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, n, 0x10, 0x20, 0x30, 0x40];
        v.extend_from_slice(&[0xFF, 0xD9]);
        v
    };
    let f1 = make(1);
    let f2 = make(2);
    let f3 = make(3);

    // 1) 垃圾前缀 + 完整帧（单包）
    let mut junk = vec![0xDE, 0xAD, 0xBE, 0xEF];
    junk.extend_from_slice(&f1);
    sock.send(&junk).await.unwrap();

    // 2) 跨包重组：f2 拆成两段发送
    let mid = f2.len() / 2;
    sock.send(&f2[..mid]).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    sock.send(&f2[mid..]).await.unwrap();

    // 3) 单包多帧：f3 + f1
    let mut both = f3.clone();
    both.extend_from_slice(&f1);
    sock.send(&both).await.unwrap();

    // 等接收线程处理完 4 帧
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let st = rx.stats();
        if st.frames >= 4 || std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let st = rx.stats();
    assert_eq!(st.frames, 4, "应解析出 4 帧，实际 {}", st.frames);
    assert!(st.bytes > 0);
    let last = rx.latest_frame().expect("应有最新帧");
    assert_eq!(last.seq, 4);
    assert_eq!(last.jpeg, f1, "最后一帧应为单包多帧中的 f1");
    assert!(last.ts_ms > 0);
    assert!(rx.is_running());
    rx.stop();
}

/// 视频（P3-2）：停止后不再接收新帧
#[tokio::test]
async fn video_receiver_stop() {
    use groundctrl_core::services::video::VideoReceiver;
    use tokio::net::UdpSocket;

    let rx = VideoReceiver::start("127.0.0.1:0").await.unwrap();
    let target = rx.local_addr();
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    sock.connect(target).await.unwrap();

    let frame = vec![0xFF, 0xD8, 0x01, 0xFF, 0xD9];
    sock.send(&frame).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(rx.stats().frames, 1);

    rx.stop();
    assert!(!rx.is_running());
    let before = rx.stats().frames;
    sock.send(&frame).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(rx.stats().frames, before, "停止后不应再接收");
}

/// 脚本（P3-4）：解析覆盖全部语句类型
#[test]
fn script_parse_all_stmt_types() {
    use groundctrl_core::services::script::{parse_script, IfCond, Stmt};

    let text = "\
# 示例任务
WAIT 2.5
SET_MODE GUIDED
ARM
DISARM
TAKEOFF 10
LAND
RTL
GOTO_WP 3
SET_PARAM THR_MAX 900
LOG hello world
IF BATTERY_LT 30
  LOG 低电量
  RTL
END
IF ARMED
  DISARM
END
IF GPS_LT 3
  LOG 无 3D 定位
END
";
    let prog = parse_script("t", text).expect("解析应成功");
    let s = &prog.stmts;
    assert_eq!(s.len(), 13, "顶层应有 13 条语句");
    assert_eq!(s[0], Stmt::WaitSecs(2.5));
    assert_eq!(s[1], Stmt::SetMode(4));
    assert_eq!(s[2], Stmt::Arm);
    assert_eq!(s[3], Stmt::Disarm);
    assert_eq!(s[4], Stmt::Takeoff(10.0));
    assert_eq!(s[5], Stmt::Land);
    assert_eq!(s[6], Stmt::Rtl);
    assert_eq!(s[7], Stmt::GotoWp(3));
    assert_eq!(
        s[8],
        Stmt::SetParam("THR_MAX".to_string(), 900.0)
    );
    assert_eq!(s[9], Stmt::Log("hello world".to_string()));
    assert!(matches!(&s[10], Stmt::If(IfCond::BatteryLt(30.0), sub) if sub.len() == 2));
    assert!(matches!(&s[11], Stmt::If(IfCond::Armed, sub) if sub.len() == 1));
    assert!(matches!(&s[12], Stmt::If(IfCond::GpsLt(3), sub) if sub.len() == 1));
}

/// 脚本（P3-4）：执行链路（SimLink 回执 + 条件分支）
#[tokio::test]
async fn script_runs_over_simlink() {
    use groundctrl_core::services::script::{parse_script, run_script, ScriptAbort, ScriptEvent};
    use groundctrl_core::vehicle::VehicleModel;

    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    let prog = parse_script(
        "demo",
        "\
LOG 任务开始
IF BATTERY_LT 50
  LOG 电量低
  RTL
END
DISARM
",
    )
    .expect("解析应成功");

    // 确定性车辆快照：电量 25% < 50 → IF 分支应执行
    let mut v = VehicleModel::default();
    v.battery.remaining_pct = Some(25);

    let abort = ScriptAbort::new();
    let mut events: Vec<ScriptEvent> = Vec::new();
    run_script(&hub, || Some(v.clone()), &prog, &abort, &mut |ev| events.push(ev))
        .await
        .expect("脚本应运行成功");

    // 日志事件应包含 IF 分支（RTL 已执行）
    let logs: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            ScriptEvent::Log(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(logs.contains(&"任务开始".to_string()), "应有开头日志: {logs:?}");
    assert!(logs.contains(&"电量低".to_string()), "低电量分支应执行: {logs:?}");
    assert!(
        events.iter().any(|e| matches!(e, ScriptEvent::Step(_, d) if d.contains("RTL"))),
        "应记录 RTL 步骤: {events:?}"
    );

    // 最后一条指令 DISARM(400) 的回执
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let fleet = hub.fleet().await;
    let v = fleet.first().expect("机队非空");
    let ack = v.cmd_ack.as_ref().expect("应有回执");
    assert_eq!(ack.command, 400, "最后指令应为 DISARM");
    assert_eq!(ack.result, 0, "SimLink 应回 ACCEPTED");
}

/// 脚本（P3-4）：条件为假跳过分支 + 中止信号生效
#[tokio::test]
async fn script_skip_branch_and_abort() {
    use groundctrl_core::services::script::{parse_script, run_script, ScriptAbort, ScriptEvent};
    use groundctrl_core::vehicle::VehicleModel;

    let hub = TelemetryHub::new();
    hub.connect(share(SimLink::new())).await;

    // 电量 80 → IF 条件为假，跳过 RTL
    let mut v = VehicleModel::default();
    v.battery.remaining_pct = Some(80);
    let prog = parse_script("demo", "IF BATTERY_LT 50\nRTL\nEND\nDISARM").unwrap();
    let abort = ScriptAbort::new();
    let mut logs: Vec<String> = Vec::new();
    run_script(&hub, || Some(v.clone()), &prog, &abort, &mut |ev| {
        if let ScriptEvent::Log(t) = ev {
            logs.push(t);
        }
    })
    .await
    .expect("运行成功");
    assert!(logs.is_empty(), "IF 条件为假时分支不应执行: {logs:?}");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let fleet = hub.fleet().await;
    let ack = fleet.first().and_then(|x| x.cmd_ack.clone());
    let ack = ack.expect("应有回执");
    assert_eq!(ack.command, 400, "跳过 RTL 后最后指令应为 DISARM");

    // 中止：WAIT 30 应在 200ms 内被提前终止
    let prog2 = parse_script("slow", "WAIT 30\nRTL").unwrap();
    let abort2 = ScriptAbort::new();
    let ab = abort2.clone();
    let t = tokio::spawn(async move { run_script(&hub, || None, &prog2, &ab, &mut |_| {}).await });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    abort2.abort();
    let r = t.await.unwrap();
    assert!(r.is_err(), "中止后应返回错误");
    assert!(r.unwrap_err().to_string().contains("中止"), "错误应说明已中止");
}

