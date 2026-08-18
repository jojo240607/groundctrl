// 用 groundctrl-core 的 MavlinkParser（已修复 PARAM_VALUE 兼容解析）连 COM12，
// 发 PARAM_REQUEST_LIST，统计 PARAM_VALUE 是否能被解析出来。
use std::time::Duration;
use groundctrl_core::link::serial::{SerialConfig, SerialLink};
use groundctrl_core::link::Link;
use groundctrl_core::mlink::{self, MavlinkParser};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let port = std::env::var("PORT").unwrap_or_else(|_| "COM12".into());
    let baud = 115_200u32;
    let serial = SerialLink::new(SerialConfig { port: port.clone(), baud_rate: baud });

    // 等待串口后台任务真正打开端口（tokio_serial open 是异步的，发送前必须就绪，
    // 否则字节只进通道、open 失败则全丢，导致板子收不到命令）。
    let mut waited = 0;
    while !serial.is_open() && waited < 100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        waited += 1;
    }
    if !serial.is_open() {
        eprintln!("ERROR: serial {port} failed to open (is the board connected? is the port in use by another tool?)");
        std::process::exit(2);
    }
    println!("serial {port} opened");

    // 发 PARAM_REQUEST_LIST（标准顺序编码）
    let out = mlink::encode_std_param_request_list(1, 1, 0);
    serial.send(&out).await.unwrap();
    println!("sent PARAM_REQUEST_LIST ({} bytes): {}", out.len(), out.iter().map(|b| format!("{:02x}", b)).collect::<String>());

    // 也发 COMMAND_LONG REQUEST_AUTOPILOT_CAPABILITIES 验证上行链路（标准顺序编码）
    let out2 = mlink::encode_std_command_long(
        1, 1, 0,
        mavlink::common::MavCmd::MAV_CMD_REQUEST_AUTOPILOT_CAPABILITIES as u16,
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        0,
    );
    serial.send(&out2).await.unwrap();
    println!("sent COMMAND_LONG CAP(520) ({} bytes): {}", out2.len(), out2.iter().map(|b| format!("{:02x}", b)).collect::<String>());

    // 也发 ARM 命令(400) 验证解锁链路
    let out3 = mlink::encode_std_command_long(
        1, 1, 0,
        mavlink::common::MavCmd::MAV_CMD_COMPONENT_ARM_DISARM as u16,
        [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        0,
    );
    serial.send(&out3).await.unwrap();
    println!("sent COMMAND_LONG ARM(400) ({} bytes): {}", out3.len(), out3.iter().map(|b| format!("{:02x}", b)).collect::<String>());

    let mut parser = MavlinkParser::new();
    let mut pv_count = 0usize;
    let mut cap_count = 0usize;
    let mut ack_count = 0usize;
    // 原始字节扫描：MAVLink v2 头始终是标准布局（byte5-7=msgid LE），
    // 不受 crate 字段顺序 bug 影响，用来判断板子是否回 COMMAND_ACK(77)。
    let mut msgid_hist: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(6) {
        let chunk = match serial.recv().await {
            Ok(c) => c,
            Err(e) => {
                if start.elapsed() >= Duration::from_secs(6) { break; }
                continue;
            }
        };
        // 原始扫描 msgid
        let mut i = 0;
        while i + 9 < chunk.len() {
            if chunk[i] == 0xFD {
                let len = chunk[i + 1] as usize;
                let msgid = (chunk[i + 7] as u32)
                    | ((chunk[i + 8] as u32) << 8)
                    | ((chunk[i + 9] as u32) << 16);
                let total = 10 + len + 2;
                if i + total <= chunk.len() {
                    *msgid_hist.entry(msgid).or_insert(0) += 1;
                    i += total;
                    continue;
                }
            }
            i += 1;
        }
        let msgs = parser.feed(&chunk).unwrap();
        for (_h, m) in msgs {
            if let mavlink::common::MavMessage::PARAM_VALUE(d) = m {
                let id = String::from_utf8_lossy(&d.param_id)
                    .trim_end_matches('\0').to_string();
                println!("  PARAM_VALUE: id={} value={} count={} index={}",
                    id, d.param_value, d.param_count, d.param_index);
                pv_count += 1;
            } else if let mavlink::common::MavMessage::AUTOPILOT_VERSION(_) = m {
                println!("  AUTOPILOT_VERSION received");
                cap_count += 1;
            } else if let mavlink::common::MavMessage::COMMAND_ACK(_) = m {
                println!("  COMMAND_ACK received");
                ack_count += 1;
            }
        }
    }
    println!("total PARAM_VALUE parsed = {}, AUTOPILOT_VERSION = {}, COMMAND_ACK = {}", pv_count, cap_count, ack_count);
    println!("raw msgid histogram (from v2 headers):");
    let mut keys: Vec<_> = msgid_hist.keys().cloned().collect();
    keys.sort();
    for k in keys {
        println!("  msgid {:>3} (0x{:02X}): {}", k, k, msgid_hist[&k]);
    }
}
