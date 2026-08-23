// 用共用 mavlink-core 直接解析已知正确的 PARAM_VALUE 帧 hex，验证编解码一致性。
use mavlink_core::common::{MavHeader, MavMessage, MavParamType, read_v2_msg, write_v2_msg};

fn main() {
    // 从 diag_parse 捕获的真实帧（plen=25, index=0, KpXY=0.5）
    let hex = "fd1900003b01011600004b7058590000000000000000000000000000003f09050000000b3d";
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    println!("frame len = {}", bytes.len());
    match read_v2_msg(&bytes) {
        Some((header, msg, consumed)) => {
            println!("PARSE OK header={:?} consumed={}", header, consumed);
            if let MavMessage::PARAM_VALUE(d) = msg {
                let id = String::from_utf8_lossy(&d.param_id).trim_end_matches('\0').to_string();
                println!(
                    "PARAM_VALUE: id={} value={} type={:?} count={} index={}",
                    id, d.param_value, d.param_type, d.param_count, d.param_index
                );
            } else {
                println!("parsed but not PARAM_VALUE: {:?}", msg);
            }
        }
        None => println!("PARSE FAIL"),
    }

    // 再用 write_v2_msg 生成一个标准 PARAM_VALUE 帧，对比 hex
    let header = MavHeader { system_id: 1, component_id: 1, sequence: 0 };
    let msg = MavMessage::PARAM_VALUE(mavlink_core::common::PARAM_VALUE_DATA {
        param_id: *b"KpXY\0\0\0\0\0\0\0\0\0\0\0\0",
        param_value: 0.5,
        param_type: MavParamType::MAV_PARAM_TYPE_REAL32,
        param_count: 5,
        param_index: 0,
    });
    match write_v2_msg(&header, &msg) {
        Ok(out) => {
            let hex: String = out.iter().map(|b| format!("{:02x}", b)).collect();
            println!("generated: {}", hex);
        }
        Err(e) => println!("WRITE FAIL: {:?}", e),
    }
}
