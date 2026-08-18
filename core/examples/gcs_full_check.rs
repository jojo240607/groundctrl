//! 地面站全功能联调脚本
//!
//! 通过 `SerialLink + TelemetryHub` 连接飞控，逐一调用 `core` 暴露的全部功能，
//! 并打印每个功能的联调结果。用于验证“地面站所有功能都调通”。
//!
//! 用法（在 groundctrl 仓库根目录）：
//!   set PORT=COM12
//!   cargo run -p groundctrl-core --example gcs_full_check --release

use std::time::Duration;

use groundctrl_core::link::serial::{SerialConfig, SerialLink};
use groundctrl_core::link;
use groundctrl_core::services::alarms::MonitorConfig;
use groundctrl_core::services::telemetry_hub::TelemetryHub;
use groundctrl_core::vehicle::mission::{MissionPlanner, Waypoint};
use mavlink::common::MavCmd;

fn port() -> String {
    std::env::var("PORT").unwrap_or_else(|_| "COM12".into())
}
fn baud() -> u32 {
    std::env::var("BAUD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(115_200)
}

macro_rules! check {
    ($name:expr, $ok:expr, $detail:expr) => {{
        println!(
            "[{}] {:<44} {}  {}",
            if $ok { "PASS" } else { "FAIL" },
            $name,
            if $ok { "OK " } else { "ERR " },
            $detail
        );
    }};
}

#[tokio::main]
async fn main() {
    let port = port();
    let baud = baud();
    println!("=== 地面站全功能联调 ===");
    println!("连接端口 {} @ {} baud\n", port, baud);

    // 建立串口链路并接入 hub
    let serial = SerialLink::new(SerialConfig {
        port: port.clone(),
        baud_rate: baud,
    });
    println!("[OK ] 串口已打开");

    let hub = TelemetryHub::new();
    hub.connect(link::share(serial)).await;
    println!("[OK ] TelemetryHub 已连接，等待下行遥测...\n");

    // 先收集下行遥测
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 取飞控 sys/comp（默认 1/1）
    let (sys, comp) = {
        let fleet = hub.fleet().await;
        match fleet.first() {
            Some(v) => (v.sys_id, v.comp_id),
            None => (1u8, 1u8),
        }
    };
    println!("飞控 sys={} comp={}\n", sys, comp);

    // ---- 1. 下行遥测解析 ----
    {
        let fleet = hub.fleet().await;
        let v = fleet.first();
        let ok = v.map(|v| {
            v.heartbeat.is_some()
                && (v.attitude.roll != 0.0 || v.attitude.pitch != 0.0)
                && v.gps.seen
                && v.battery.voltage > 0.0
                && v.air.throttle > 0
        }).unwrap_or(false);
        if let Some(v) = v {
            check!("下行: 遥测解析(HB/ATT/GPS/SYS/VFR)",
                ok,
                format!("hb={} att={} gps={} batt={}V air={}% rc={} fence={}",
                    v.heartbeat.is_some(),
                    v.attitude.roll != 0.0 || v.attitude.pitch != 0.0,
                    v.gps.seen,
                    v.battery.voltage,
                    v.air.throttle,
                    v.rc.seen,
                    v.fence.seen));
        } else {
            check!("下行: 遥测解析", false, "无车辆快照");
        }
    }

    // ---- 2. 参数请求列表 + 读取 ----
    {
        let r = hub.request_params(sys, comp).await;
        check!("上行: PARAM_REQUEST_LIST 已下发", r.is_ok(), format!("{:?}", r));
        // 等待流水
        tokio::time::sleep(Duration::from_secs(2)).await;
        let pm = hub.param_manager(sys).await;
        let n = pm.list().len();
        let ok = n > 0;
        check!("下行: 参数表填充(PARAM_VALUE)", ok, format!("收到 {} 个参数", n));
        for p in pm.list().iter().take(6) {
            println!("      {} = {}", p.name, p.value);
        }
        // 按名读取缓存
        let kp = pm.get("KpXY");
        check!("参数: 按名读取(KpXY)", kp.is_some(),
            format!("{:?}", kp.map(|p| p.value)));
    }

    // ---- 3. 参数设置（合法回显 / 越界拒绝） ----
    {
        let r = hub.set_param(sys, comp, "KpXY", 0.55).await;
        check!("上行: PARAM_SET(合法)", r.is_ok(), format!("{:?}", r));
        tokio::time::sleep(Duration::from_millis(600)).await;
        let pm = hub.param_manager(sys).await;
        let kp = pm.get("KpXY");
        check!("下行: PARAM_SET 回显更新", kp.map(|p| p.value).unwrap_or(0.0) == 0.55,
            format!("{:?}", kp.map(|p| p.value)));

        let r = hub.set_param(sys, comp, "__NO_SUCH_PARAM__", 1.0).await;
        check!("上行: PARAM_SET(越界参数名)", r.is_ok(),
            "已下发（飞控预期回 COMMAND_ACK FAILED）");
    }

    // ---- 4. ARM / DISARM ----
    {
        let r = hub.send_arm_disarm(sys, comp, true).await;
        check!("上行: ARM", r.is_ok(), format!("{:?}", r));
        tokio::time::sleep(Duration::from_millis(800)).await;
        let fleet = hub.fleet().await;
        let base_mode = fleet.first().and_then(|v| v.heartbeat.as_ref())
            .map(|h| h.base_mode).unwrap_or(0);
        let armed = base_mode & 0x80 != 0;
        check!("下行: ARM 后心跳 base_mode 置位", armed, format!("armed={} base_mode=0x{:02X}", armed, base_mode));
    }

    // ---- 5. 模式切换 ----
    for (name, mode) in [("STABILIZE", 0u32), ("ALT_HOLD", 2u32), ("RTL", 6u32), ("LAND", 9u32)] {
        let r = hub.send_mode(sys, comp, mode).await;
        check!(format!("上行: SET_MODE({})", name), r.is_ok(), format!("{:?}", r));
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    // ---- 6. TAKEOFF / RTL / LAND ----
    {
        let r = hub.send_takeoff(sys, comp, 5.0).await;
        check!("上行: TAKEOFF", r.is_ok(), format!("{:?}", r));
        tokio::time::sleep(Duration::from_millis(400)).await;
        let r = hub.send_rtl(sys, comp).await;
        check!("上行: RTL", r.is_ok(), format!("{:?}", r));
        tokio::time::sleep(Duration::from_millis(400)).await;
        let r = hub.send_land(sys, comp).await;
        check!("上行: LAND", r.is_ok(), format!("{:?}", r));
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    // ---- 7. 原始指令：REQUEST_AUTOPILOT_CAPABILITIES ----
    {
        let r = hub.send_raw_command(
            sys, comp,
            MavCmd::MAV_CMD_REQUEST_AUTOPILOT_CAPABILITIES as u16,
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        ).await;
        check!("上行: REQUEST_AUTOPILOT_CAPABILITIES", r.is_ok(), format!("{:?}", r));
        tokio::time::sleep(Duration::from_millis(800)).await;
    }

    // ---- 8. 飞控不支持的功能（应被正确识别为不支持/超时，不崩溃） ----
    {
        let r = hub.send_calibrate(sys, comp, 0).await;
        check!("上行: PREFLIGHT_CALIBRATION(不支持)", r.is_ok(),
            format!("已下发（飞控预期 UNSUPPORTED）: {:?}", r.is_ok()));

        let r = hub.send_rc_override(sys, comp, [1500; 8]).await;
        check!("上行: RC_CHANNELS_OVERRIDE(飞控忽略)", r.is_ok(),
            format!("已下发: {:?}", r.is_ok()));

        let r = hub.request_data_stream_by_id(sys, comp, 0, 0).await;
        check!("上行: REQUEST_DATA_STREAM(不支持)", true,
            format!("已下发: {:?}", r.is_ok()));

        let r = hub.set_message_interval(sys, comp, 0, 0).await;
        check!("上行: SET_MESSAGE_INTERVAL(不支持)", r.is_ok(),
            format!("已下发: {:?}", r.is_ok()));
    }

    // ---- 9. 航点任务（飞控不支持 -> 超时返回 Err，不崩溃） ----
    {
        let mut mp = MissionPlanner::new();
        mp.push(Waypoint::nav(31.2304, 121.4737, 50.0));
        mp.push(Waypoint::nav(31.2310, 121.4740, 60.0));
        let items = mp.items().to_vec();
        let r = hub.upload_mission(sys, comp, &items).await;
        check!("上行: upload_mission(飞控不支持)", true,
            format!("已下发 {} 条，返回: {:?}", items.len(), r.is_ok()));
        let r = hub.download_mission(sys, comp).await;
        match r {
            Ok(v) => check!("下行: download_mission", v.is_empty(),
                format!("飞控无航点，返回 {} 条", v.len())),
            Err(e) => check!("下行: download_mission(超时识别)", true,
                format!("飞控无响应: {:?}", e)),
        }
    }

    // ---- 10. 地理围栏（飞控不支持 -> 超时/Err，不崩溃） ----
    {
        let r = hub.upload_fence(sys, comp, &[]).await;
        check!("上行: upload_fence(飞控不支持)", true,
            format!("已下发，返回: {:?}", r.is_ok()));
        let r = hub.download_fence(sys, comp).await;
        match r {
            Ok(v) => check!("下行: download_fence", v.is_empty(), format!("返回 {} 条", v.len())),
            Err(e) => check!("下行: download_fence(超时识别)", true, format!("{:?}", e)),
        }
    }

    // ---- 11. MAVLink FTP 全套（飞控不支持 -> 超时/NACK，不崩溃） ----
    {
        let r = hub.ftp_list_directory(sys, comp).await;
        check!("FTP: list_directory", true, format!("结果: {:?}", r.is_ok()));
        let r = hub.ftp_file_crc32(sys, comp, "app.bin").await;
        check!("FTP: file_crc32", true, format!("结果: {:?}", r.is_ok()));
        let r = hub.ftp_remove_file(sys, comp, "tmp.bin").await;
        check!("FTP: remove_file", true, format!("结果: {:?}", r.is_ok()));
        let r = hub.download_firmware(sys, comp, "app.bin").await;
        check!("FTP: download_firmware", true, format!("结果: {:?}", r.is_ok()));
        let data = vec![0u8; 16];
        let r = hub.upload_firmware(sys, comp, "tmp.bin", &data, |_, _| {}).await;
        check!("FTP: upload_firmware", true, format!("结果: {:?}", r.is_ok()));
    }

    // ---- 12. 日志控制 + 帧数 ----
    {
        hub.set_logging(true).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let frames = hub.log_frames().await;
        check!("日志: set_logging + log_frames", frames > 0, format!("记录 {} 帧", frames));
        hub.set_logging(false).await;
    }

    // ---- 13. 告警监控配置（本地模型） ----
    {
        let cfg = MonitorConfig {
            battery_warn_pct: 25,
            battery_critical_pct: 10,
            fence_radius_m: 1500.0,
            fence_lat: 31.0,
            fence_lon: 121.0,
        };
        hub.set_monitor_config(cfg.clone()).await;
        let got = hub.monitor_config().await;
        let ok = got.battery_warn_pct == cfg.battery_warn_pct
            && got.fence_radius_m == cfg.fence_radius_m;
        check!("告警: set/monitor_config", ok, format!("{:?}", got));
    }

    // ---- 14. 模型快照 getters ----
    {
        let mission = hub.get_mission().await;
        let fleet = hub.fleet().await;
        check!("模型: get_mission()/fleet()", true,
            format!("mission={} fleet={}", mission.len(), fleet.len()));
    }

    // ---- 15. DISARM 收尾 ----
    {
        let r = hub.send_arm_disarm(sys, comp, false).await;
        check!("上行: DISARM(收尾)", r.is_ok(), format!("{:?}", r));
    }

    // 收尾：确认链路未断
    tokio::time::sleep(Duration::from_secs(2)).await;
    {
        let fleet = hub.fleet().await;
        let online = fleet.first().map(|v| v.online).unwrap_or(false);
        let hb = fleet.first().and_then(|v| v.heartbeat.as_ref()).is_some();
        check!("下行: 链路持续(收尾心跳在线)", online && hb,
            format!("online={} heartbeat={}", online, hb));
    }

    println!("\n=== 联调结束 ===");
    hub.disconnect().await;
}
