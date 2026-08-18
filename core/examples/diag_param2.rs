// 用标准 mavlink crate 直接解析已知正确的 PARAM_VALUE 帧 hex，验证 crate 能否解析。
use std::io::Cursor;
use mavlink::common::{MavMessage, MavParamType};
use mavlink::{read_v2_msg, write_v2_msg, MavHeader, MavlinkVersion};

fn main() {
    // 从 diag_parse 捕获的真实帧（plen=25, index=0, KpXY=0.5）
    let hex = "fd1900003b01011600004b7058590000000000000000000000000000003f09050000000b3d";
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    println!("frame len = {}", bytes.len());
    let mut cur = Cursor::new(bytes);
    match read_v2_msg::<MavMessage, _>(&mut cur) {
        Ok((header, msg)) => {
            println!("PARSE OK header={:?}", header);
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
        Err(e) => println!("PARSE FAIL: {:?}", e),
    }

    // 再用 write_v2_msg 生成一个标准 PARAM_VALUE 帧，对比 hex
    let header = MavHeader { system_id: 1, component_id: 1, sequence: 0 };
    let msg = MavMessage::PARAM_VALUE(mavlink::common::PARAM_VALUE_DATA {
        param_id: *b"KpXY\0\0\0\0\0\0\0\0\0\0\0\0",
        param_value: 0.5,
        param_type: MavParamType::MAV_PARAM_TYPE_REAL32,
        param_count: 5,
        param_index: 0,
    });
    let mut out = Vec::new();
    match write_v2_msg(&mut out, header, &msg) {
        Ok(_) => {
            let hex: String = out.iter().map(|b| format!("{:02x}", b)).collect();
            println!("generated: {}", hex);
        }
        Err(e) => println!("WRITE FAIL: {:?}", e),
    }
}
