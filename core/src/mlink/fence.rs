//! 地理围栏消息族的手写 MAVLink v2 编解码
//!
//! `FENCE_POINT`(160) / `FENCE_FETCH_POINT`(161) 仅存在于 ardupilotmega dialect，
//! 而本项目统一使用 `common` dialect（见 mlink/mod.rs 说明，避免破坏 MavMessage
//! 泛型约束）。因此这里按 MAVLink v2 协议手写这两个消息的编码与解析，
//! CRC 采用 MAVLink 标准 CRC-16/MCRF4XX（extra_crc 常量取自 ArduPilot 官方生成头）：
//! - FENCE_POINT       msg 160, crc_extra 78
//! - FENCE_FETCH_POINT msg 161, crc_extra 68
//!
//! 同步可解析的 `FENCE_STATUS`(162) 仍在 common dialect 中，由正常解析链路处理。

use crate::error::{GcError, Result};

pub const FENCE_POINT_MSG_ID: u32 = 160;
pub const FENCE_FETCH_POINT_MSG_ID: u32 = 161;
const FENCE_POINT_CRC_EXTRA: u8 = 78;
const FENCE_FETCH_POINT_CRC_EXTRA: u8 = 68;

/// 解析出的围栏点（坐标单位 1e7 度，与 MAVLink int32 一致）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FencePointRaw {
    pub target_system: u8,
    pub target_component: u8,
    pub idx: u8,
    pub count: u8,
    pub lat: i32, // 1e7 deg
    pub lng: i32, // 1e7 deg
}

/// 解析出的围栏点请求（FENCE_FETCH_POINT）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FenceFetchRaw {
    pub target_system: u8,
    pub target_component: u8,
    pub idx: u8,
}

/// MAVLink 帧 CRC（CRC-16/MCRF4XX：poly 0x1021, init 0xFFFF, 输入/输出均反射, xorout 0）
pub fn crc16_mcrf4cc(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            if crc & 1 != 0 {
                // 反射后的多项式 0x8408
                crc = (crc >> 1) ^ 0x8408;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

/// 组装一条 MAVLink v2 帧（不校验消息是否在 common dialect 中）
fn build_v2_frame(
    header: &crate::mlink::MavHeader,
    msg_id: u32,
    payload: &[u8],
    crc_extra: u8,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(10 + payload.len() + 2);
    buf.push(0xFD); // MAV_STX_V2
    buf.push(payload.len() as u8);
    buf.push(0); // incompat_flags
    buf.push(0); // compat_flags
    buf.push(header.sequence);
    buf.push(header.system_id);
    buf.push(header.component_id);
    buf.extend_from_slice(&msg_id.to_le_bytes()[..3]);
    buf.extend_from_slice(payload);
    // CRC 覆盖：v2 头部（不含 STX）+ payload + extra_crc
    let mut crc_input = Vec::with_capacity(buf.len());
    crc_input.extend_from_slice(&buf[1..]);
    crc_input.push(crc_extra);
    let crc = crc16_mcrf4cc(&crc_input);
    buf.extend_from_slice(&crc.to_le_bytes());
    buf
}

/// 编码一条 FENCE_POINT（围栏点上传，坐标单位 1e7 度）
pub fn encode_fence_point(
    header: &crate::mlink::MavHeader,
    target_system: u8,
    target_component: u8,
    idx: u8,
    count: u8,
    lat_e7: i32,
    lng_e7: i32,
) -> Result<Vec<u8>> {
    if count == 0 {
        return Err(GcError::Mavlink("围栏点数不能为 0".into()));
    }
    let mut payload = Vec::with_capacity(12);
    payload.push(target_system);
    payload.push(target_component);
    payload.push(idx);
    payload.push(count);
    payload.extend_from_slice(&lat_e7.to_le_bytes());
    payload.extend_from_slice(&lng_e7.to_le_bytes());
    Ok(build_v2_frame(
        header,
        FENCE_POINT_MSG_ID,
        &payload,
        FENCE_POINT_CRC_EXTRA,
    ))
}

/// 编码一条 FENCE_FETCH_POINT（请求指定索引的围栏点）
pub fn encode_fence_fetch_point(
    header: &crate::mlink::MavHeader,
    target_system: u8,
    target_component: u8,
    idx: u8,
) -> Result<Vec<u8>> {
    let mut payload = Vec::with_capacity(3);
    payload.push(target_system);
    payload.push(target_component);
    payload.push(idx);
    Ok(build_v2_frame(
        header,
        FENCE_FETCH_POINT_MSG_ID,
        &payload,
        FENCE_FETCH_POINT_CRC_EXTRA,
    ))
}

/// 从原始 v2 帧中提取 (payload, crc_extra)，并校验 CRC
fn parse_v2_frame<'a>(bytes: &'a [u8], expect_msg_id: u32, crc_extra: u8) -> Option<&'a [u8]> {
    // 最短帧：STX + 9B 头 + 0 负载 + 2B CRC = 12
    if bytes.len() < 12 || bytes[0] != 0xFD {
        return None;
    }
    let payload_len = bytes[1] as usize;
    let total = 10 + payload_len + 2;
    if bytes.len() < total {
        return None;
    }
    let msg_id = u32::from_le_bytes([bytes[7], bytes[8], bytes[9], 0]);
    if msg_id != expect_msg_id {
        return None;
    }
    let mut crc_input = Vec::with_capacity(9 + payload_len + 1);
    crc_input.extend_from_slice(&bytes[1..10 + payload_len]);
    crc_input.push(crc_extra);
    let crc = crc16_mcrf4cc(&crc_input);
    let wire_crc = u16::from_le_bytes([bytes[total - 2], bytes[total - 1]]);
    if crc != wire_crc {
        return None;
    }
    Some(&bytes[10..10 + payload_len])
}

/// 解析 FENCE_POINT 帧
pub fn decode_fence_point(bytes: &[u8]) -> Option<FencePointRaw> {
    let payload = parse_v2_frame(bytes, FENCE_POINT_MSG_ID, FENCE_POINT_CRC_EXTRA)?;
    if payload.len() != 12 {
        return None;
    }
    Some(FencePointRaw {
        target_system: payload[0],
        target_component: payload[1],
        idx: payload[2],
        count: payload[3],
        lat: i32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]),
        lng: i32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]),
    })
}

/// 解析 FENCE_FETCH_POINT 帧
pub fn decode_fence_fetch_point(bytes: &[u8]) -> Option<FenceFetchRaw> {
    let payload = parse_v2_frame(bytes, FENCE_FETCH_POINT_MSG_ID, FENCE_FETCH_POINT_CRC_EXTRA)?;
    if payload.len() != 3 {
        return None;
    }
    Some(FenceFetchRaw {
        target_system: payload[0],
        target_component: payload[1],
        idx: payload[2],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mavlink_core::common as mav;
    use crate::mlink;

    /// 交叉验证：手写 CRC 与 mavlink crate 对 FENCE_STATUS(162) 的计算一致
    #[test]
    fn crc_matches_mavlink_crate() {
        let msg = mlink::MavMessage::FENCE_STATUS(mav::FENCE_STATUS_DATA {
            breach_time: 1234,
            breach_count: 5,
            breach_status: 1,
            breach_type: mav::FenceBreach::FENCE_BREACH_BOUNDARY,
        });
        let header = mlink::default_header();
        let bytes = mlink::encode_v2(&header, &msg).expect("encode");

        // 从帧尾取出 crate 计算的 CRC
        let payload_len = bytes[1] as usize;
        let total = 10 + payload_len + 2;
        let wire_crc = u16::from_le_bytes([bytes[total - 2], bytes[total - 1]]);

        // 用手写算法重算（头部 + payload + extra_crc）
        let mut input = Vec::new();
        input.extend_from_slice(&bytes[1..10 + payload_len]);
        input.push(mlink::MavMessage::extra_crc(162));
        assert_eq!(crc16_mcrf4cc(&input), wire_crc, "CRC 算法应与 mavlink crate 一致");
    }

    /// FENCE_POINT / FENCE_FETCH_POINT 编码 -> 解析往返一致
    #[test]
    fn fence_point_roundtrip() {
        let header = crate::mlink::MavHeader {
            system_id: 255,
            component_id: 190,
            sequence: 7,
        };
        let bytes = encode_fence_point(&header, 1, 1, 2, 4, 311_234_567, 121_876_543).unwrap();
        let raw = decode_fence_point(&bytes).expect("should parse back");
        assert_eq!(
            raw,
            FencePointRaw {
                target_system: 1,
                target_component: 1,
                idx: 2,
                count: 4,
                lat: 311_234_567,
                lng: 121_876_543,
            }
        );

        let fetch = encode_fence_fetch_point(&header, 1, 1, 3).unwrap();
        let f = decode_fence_fetch_point(&fetch).expect("should parse back");
        assert_eq!(
            f,
            FenceFetchRaw {
                target_system: 1,
                target_component: 1,
                idx: 3,
            }
        );
    }

    /// 篡改一字节后应无法解析（CRC 校验生效）
    #[test]
    fn fence_point_rejects_tampered() {
        let header = mlink::default_header();
        let mut bytes = encode_fence_point(&header, 1, 1, 0, 3, 311_000_000, 121_000_000).unwrap();
        bytes[12] ^= 0xFF; // 篡改 payload 首字节
        assert!(decode_fence_point(&bytes).is_none(), "篡改后 CRC 应校验失败");
    }
}
