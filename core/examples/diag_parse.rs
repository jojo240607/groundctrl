//! 诊断：用标准 mavlink crate 的 read_v2_msg 解析板子下行帧，
//! 统计每个 msg_id 解析成功/失败，定位地面站为何丢弃部分下行帧。
use std::io::Cursor;

use groundctrl_core::link::serial::{SerialConfig, SerialLink};
use groundctrl_core::link::Link;
use mavlink::common::MavMessage;

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
        let header = mavlink::MavHeader {
            system_id: 255,
            component_id: 190,
            sequence: 0,
        };
        let req = mavlink::common::MavMessage::PARAM_REQUEST_LIST(
            mavlink::common::PARAM_REQUEST_LIST_DATA {
                target_system: 1,
                target_component: 1,
            },
        );
        let mut out = Vec::new();
        mavlink::write_v2_msg(&mut out, header, &req).unwrap();
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
                    let mut cur = Cursor::new(buf.clone());
                    if msg_id == 22 && std::env::var("DUMP_PARAM").is_ok() {
                        // MAVLink v2 header is 10 bytes (magic+len+incompat+compat+seq+sys+comp+msgid3)
                        let plen = buf[1] as usize;
                        let framelen = 10 + plen + 2;
                        let hex: String = buf.iter().take(framelen).map(|b| format!("{:02x}", b)).collect();
                        println!("PARAM_VALUE raw frame (plen={}): {}", plen, hex);
                    }
                    match mavlink::read_v2_msg::<MavMessage, _>(&mut cur) {
                        Ok((_h, _m)) => {
                            let consumed = cur.position() as usize;
                            *successes.entry(msg_id).or_insert(0) += 1;
                            total += 1;
                            buf.drain(..consumed);
                        }
                        Err(_e) => {
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
