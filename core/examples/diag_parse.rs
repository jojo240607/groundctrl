//! 诊断：用共用 mavlink-core 的 read_v2_msg 解析板子下行帧，
//! 统计每个 msg_id 解析成功/失败，定位地面站为何丢弃部分下行帧。

use groundctrl_core::link::serial::{SerialConfig, SerialLink};
use groundctrl_core::link::Link;
use mavlink_core::common::MavMessage;

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT").unwrap_or_else(|_| "COM12".into());
    let serial = SerialLink::new(SerialConfig {
        port,
        baud_rate: 115_200,
    });
    let mut buf = Vec::new();
    let mut successes: std::collections::HashMap<u32, u32> = Default::default();
    let mut failures: std::collections::HashMap<u32, u32> = Default::default();
    let start = std::time::Instant::now();
    let mut total = 0u32;
    // 开头发一个 PARAM_REQUEST_LIST（sys=1, comp=1），让飞控回 PARAM_VALUE(22)
    {
        let header = mavlink_core::common::MavHeader {
            system_id: 255,
            component_id: 190,
            sequence: 0,
        };
        let req = MavMessage::PARAM_REQUEST_LIST(mavlink_core::common::PARAM_REQUEST_LIST_DATA {
            target_system: 1,
            target_component: 1,
        });
        let out = mavlink_core::common::write_v2_msg(&header, &req).unwrap();
        let _ = serial.send(&out).await;
    }
    while start.elapsed().as_secs() < 12 {
        match serial.recv().await {
            Ok(chunk) => {
                let chunk: Vec<u8> = chunk;
                buf.extend_from_slice(&chunk);
                // 尝试从 buf 解析尽可能多的帧（magic 0xFD 起）
                loop {
                    // 找到下一个 magic
                    let pos = buf.iter().position(|&b| b == 0xFD);
                    if pos.is_none() {
                        if buf.len() > 4096 {
                            buf.drain(..buf.len() - 4096);
                        }
                        break;
                    }
                    buf.drain(..pos.unwrap());
                    if buf.len() < 12 {
                        break;
                    }
                    // mavlink v2 header: magic(0) len(1) incompat(2) compat(3) seq(4) sys(5) comp(6) msgid(7..10, 3 bytes LE)
                    let msg_id = (buf[7] as u32)
                        | ((buf[8] as u32) << 8)
                        | ((buf[9] as u32) << 16);
                    let parsed = mavlink_core::common::read_v2_msg(&buf);
                    match parsed {
                        Some((_h, _m, consumed)) => {
                            *successes.entry(msg_id).or_insert(0) += 1;
                            total += 1;
                            buf.drain(..consumed);
                        }
                        None => {
                            *failures.entry(msg_id).or_insert(0) += 1;
                            total += 1;
                            buf.drain(..1);
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
    println!("total attempts={}", total);
    println!("SUCCESS msg_id -> count:");
    for (k, v) in successes.iter().chain(failures.iter()).collect::<Vec<_>>() {
        let s = successes.get(k).copied().unwrap_or(0);
        let f = failures.get(k).copied().unwrap_or(0);
        println!("  msg_id={:>3}  ok={:>4}  fail={:>4}", k, s, f);
    }
}
