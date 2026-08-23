//! 无头地面站 CLI（验证用）
//!
//! 通过串口连接飞控，可选择性下发指令（ARM/DISARM/SET_MODE/PARAM_SET/
//! PARAM_REQUEST_LIST），并解析飞控下行遥测帧，统计心跳、参数、指令应答，
//! 退出时汇总。用于 §4 步骤 5 的板载上行接收功能验证。
//!
//! 用法：
//! ```text
//! groundctrl-tools --port COM9 --baud 115200 \
//!     --send-cmd ARM --send-param THR_MID=0.5 --request-params --duration 15
//! ```
//! 不传 --send-* / --request-* 时仅监听下行并打印统计。

use std::time::{Duration, Instant};

use groundctrl_core::link::{serial::{SerialConfig, SerialLink}, Link};
use groundctrl_core::mlink::{self, MavlinkParser};
use mavlink_core::common::{
    MavMessage, MavParamType, COMMAND_LONG_DATA, PARAM_REQUEST_LIST_DATA, PARAM_SET_DATA,
};
use mavlink_core::common::MavHeader;
use tracing::info;

struct Args {
    port: String,
    baud: u32,
    duration: u64,
    send_cmds: Vec<String>,
    send_params: Vec<(String, f32)>,
    request_params: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        port: "COM9".into(),
        baud: 115_200,
        duration: 15,
        send_cmds: Vec::new(),
        send_params: Vec::new(),
        request_params: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--port" => a.port = it.next().unwrap_or_default(),
            "--baud" => a.baud = it.next().and_then(|s| s.parse().ok()).unwrap_or(115_200),
            "--duration" => a.duration = it.next().and_then(|s| s.parse().ok()).unwrap_or(15),
            "--send-cmd" => {
                if let Some(v) = it.next() {
                    a.send_cmds.push(v);
                }
            }
            "--send-param" => {
                if let Some(v) = it.next() {
                    if let Some((k, val)) = v.split_once('=') {
                        if let Ok(f) = val.parse::<f32>() {
                            a.send_params.push((k.to_string(), f));
                        }
                    }
                }
            }
            "--request-params" => a.request_params = true,
            "--help" | "-h" => {
                eprintln!(
                    "usage: groundctrl-tools --port COM9 --baud 115200 [--send-cmd ARM|DISARM|SET_MODE=<n>] [--send-param NAME=VAL] [--request-params] [--duration N]"
                );
                std::process::exit(0);
            }
            _ => {}
        }
    }
    a
}

/// 把文本指令翻译成 MAVLink COMMAND_LONG 消息。
fn build_command(cmd: &str) -> Option<MavMessage> {
    let mut parts = cmd.split('=');
    let name = parts.next()?;
    let extra = parts.next();
    use mavlink_core::common::MavCmd;
    // 标准 MAVLink DO_SET_MODE：param1 = base_mode(含 CUSTOM_MODE_ENABLED 位 0x80)，param2 = custom_mode。
    // 飞控 uplink 读 param2 作为 ArduCopter custom_mode（STABILIZE=0/ALT_HOLD=2/LOITER=5/RTL=6/LAND=9）。
    let mut param1 = 0.0f32;
    let mut param2 = 0.0f32;
    let command = match name.to_uppercase().as_str() {
        "ARM" => {
            param1 = 1.0;
            MavCmd::MAV_CMD_COMPONENT_ARM_DISARM
        }
        "DISARM" => {
            param1 = 0.0;
            MavCmd::MAV_CMD_COMPONENT_ARM_DISARM
        }
        "SET_MODE" => {
            param1 = 0x80 as f32; // CUSTOM_MODE_ENABLED 位
            param2 = extra.and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
            MavCmd::MAV_CMD_DO_SET_MODE
        }
        _ => {
            eprintln!("unknown --send-cmd: {cmd} (want ARM|DISARM|SET_MODE=<n>)");
            return None;
        }
    };
    Some(MavMessage::COMMAND_LONG(COMMAND_LONG_DATA {
        target_system: 1,
        target_component: 1,
        command,
        confirmation: 0,
        param1,
        param2,
        param3: 0.0,
        param4: 0.0,
        param5: 0.0,
        param6: 0.0,
        param7: 0.0,
    }))
}

fn build_param_set(name: &str, value: f32) -> MavMessage {
    let mut id = [0u8; 16];
    let n = name.len().min(15);
    id[..n].copy_from_slice(&name.as_bytes()[..n]);
    MavMessage::PARAM_SET(PARAM_SET_DATA {
        target_system: 1,
        target_component: 1,
        param_id: id,
        param_value: value,
        param_type: MavParamType::MAV_PARAM_TYPE_REAL32,
    })
}

fn build_param_request_list() -> MavMessage {
    MavMessage::PARAM_REQUEST_LIST(PARAM_REQUEST_LIST_DATA {
        target_system: 1,
        target_component: 1,
    })
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = parse_args();
    info!(
        "headless GCS CLI: port={} baud={} duration={}s",
        args.port, args.baud, args.duration
    );

    let link = SerialLink::new(SerialConfig {
        port: args.port.clone(),
        baud_rate: args.baud,
    });

    // 等待串口真正打开。
    let start = Instant::now();
    while !link.is_open() {
        if start.elapsed() > Duration::from_secs(5) {
            eprintln!("serial {} did not open within 5s", args.port);
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    info!("serial link open");

    let header = MavHeader {
        system_id: 255,
        component_id: 190,
        sequence: 0,
    };
    let mut seq = 0u8;

    // 关键：先等 reader task 真正发起 IN token（host 读），唤醒 USB CDC 端点，
    // 否则 bulk-OUT 在 host 不读时被 Windows/设备挂起，发送的上行字节会丢失。
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 下发指令（一次性）。
    for cmd in &args.send_cmds {
        if let Some(msg) = build_command(cmd) {
            let h = MavHeader {
                system_id: header.system_id,
                component_id: header.component_id,
                sequence: seq,
            };
            if let Ok(bytes) = mlink::encode_v2(&h, &msg) {
                eprintln!("[send] cmd={} hex={}", cmd, bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
                let _ = link.send(&bytes).await;
                seq = seq.wrapping_add(1);
                info!("sent command: {cmd}");
            }
        }
    }
    for (name, value) in &args.send_params {
        let msg = build_param_set(name, *value);
        let h = MavHeader {
            system_id: header.system_id,
            component_id: header.component_id,
            sequence: seq,
        };
        if let Ok(bytes) = mlink::encode_v2(&h, &msg) {
            let _ = link.send(&bytes).await;
            seq = seq.wrapping_add(1);
            info!("sent param_set: {name}={value}");
        }
    }
    if args.request_params {
        let msg = build_param_request_list();
        let h = MavHeader {
            system_id: header.system_id,
            component_id: header.component_id,
            sequence: seq,
        };
        if let Ok(bytes) = mlink::encode_v2(&h, &msg) {
            eprintln!("[send] param_request_list hex={}", bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
            let _ = link.send(&bytes).await;
            seq = seq.wrapping_add(1);
            info!("sent param_request_list");
        }
    }

    // 监听下行并统计。
    let mut parser = MavlinkParser::new();
    let mut hb = 0u64;
    let mut param_values = 0u64;
    let mut acks = 0u64;
    let mut other = 0u64;
    let mut other_hist: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    let mut parsed = 0u64;
    let deadline = Duration::from_secs(args.duration);
    let t0 = Instant::now();

    loop {
        if t0.elapsed() >= deadline {
            break;
        }
        match link.recv().await {
            Ok(bytes) => {
                if let Ok(frames) = parser.feed(&bytes) {
                    for (hdr, msg) in frames {
                        parsed += 1;
                        match &msg {
                            MavMessage::HEARTBEAT(h) => {
                                hb += 1;
                                info!(
                                    "HEARTBEAT sys={} comp={} base_mode={:?} custom_mode={}",
                                    hdr.system_id, hdr.component_id, h.base_mode, h.custom_mode
                                );
                            }
                            MavMessage::PARAM_VALUE(p) => {
                                param_values += 1;
                                let id = String::from_utf8_lossy(&p.param_id)
                                    .trim_end_matches('\0')
                                    .to_string();
                                info!(
                                    "PARAM_VALUE {id} = {} ({}/{})",
                                    p.param_value, p.param_index, p.param_count
                                );
                            }
                            MavMessage::COMMAND_ACK(c) => {
                                acks += 1;
                                info!("COMMAND_ACK command={:?} result={:?}", c.command, c.result);
                            }
                            _ => {
                                other += 1;
                                let variant = format!("{:?}", msg);
                                let key = variant.split_whitespace().next().unwrap_or("?").to_string();
                                *other_hist.entry(key).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                // 短暂等待后重试（串口可能短暂无数据）。
                tokio::time::sleep(Duration::from_millis(20)).await;
                if t0.elapsed() >= deadline {
                    break;
                }
                let _ = e;
            }
        }
    }

    println!("==== headless GCS summary ====");
    println!("port            : {}", args.port);
    println!("frames parsed   : {parsed}");
    println!("heartbeats      : {hb}");
    println!("param_values    : {param_values}");
    println!("command_acks    : {acks}");
    println!("other messages  : {other}");
    if !other_hist.is_empty() {
        println!("other breakdown :");
        let mut items: Vec<_> = other_hist.iter().collect();
        items.sort_by(|a, b| b.1.cmp(a.1));
        for (k, v) in items.iter().take(15) {
            println!("  {k}: {v}");
        }
    }

    if hb > 0 {
        println!("RESULT: downlink OK (heartbeat received)");
    } else {
        println!("RESULT: downlink EMPTY (no heartbeat)");
    }
    if !args.send_cmds.is_empty() && acks == 0 {
        println!("WARN: commands sent but no COMMAND_ACK received");
    }
    if args.request_params && param_values == 0 {
        println!("WARN: param_request_list sent but no PARAM_VALUE received");
    }
    let _ = seq; // 序列计数器已在各帧 header 中使用
}
