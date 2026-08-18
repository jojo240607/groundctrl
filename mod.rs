//! MAVLink 服务层封装
//!
//! 选用 `common` dialect 作为默认消息类型（覆盖绝大部分飞控）。
//! 若需要厂商扩展（ardupilotmega），可在路由层按 system_id 区分。

use std::io::Cursor;

use crate::error::Result;

/// 手写编码的厂商扩展消息（围栏 FENCE_POINT / FENCE_FETCH_POINT）
pub mod fence;

/// 手写编码的 MAVLink FTP（FILE_TRANSFER_PROTOCOL, msg 110，固件升级）
pub mod ftp;

/// 项目统一消息类型（common dialect 扁平枚举，mavlink 0.11 为具体类型）
pub type MavMessage = ::mavlink::common::MavMessage;

/// 把一条消息编码为 MAVLink v2 字节帧
pub fn encode_v2(header: &::mavlink::MavHeader, msg: &MavMessage) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(280);
    ::mavlink::write_v2_msg(&mut buf, *header, msg)
        .map(|_| buf)
        .map_err(|e| crate::error::GcError::Mavlink(e.to_string()))
}

// ============================================================================
// 标准 MAVLink v2 帧手写编码
//
// 背景：`mavlink` crate 0.11.2 的 `common` dialect 中部分消息的字段序列化顺序
// 与标准 common.xml（亦为板端 flyctrl-core 所用）相反，例如 COMMAND_LONG 将
// param1 排在 command 之前、target 字段排到末尾，PARAM_VALUE 将 value 排在 id
// 之前。板端严格按标准顺序解析，导致 crate 编码的这些上行指令被板端拒绝
// （ARM / 模式切换 / 参数读写全部失效）。下行方向上恰好能正确解析的 6 种消息
// 不受影响，但所有上行控制消息都因此失灵。
//
// 这里手写标准顺序的编码函数，与板端 flyctrl-core 字节级一致，专供上行指令使用。
// ============================================================================

/// 编码一个标准 MAVLink v2 帧（10 字节头 + payload + 2 字节 CRC）。
pub fn encode_std_frame(
    msg_id: u32,
    sys_id: u8,
    comp_id: u8,
    seq: u8,
    payload: &[u8],
    crc_extra: u8,
) -> Vec<u8> {
    let mut frame = Vec::with_capacity(10 + payload.len() + 2);
    frame.push(0xFD); // magic
    frame.push(payload.len() as u8); // payload length
    frame.push(0x00); // incompat flags
    frame.push(0x00); // compat flags
    frame.push(seq); // sequence
    frame.push(sys_id); // system id
    frame.push(comp_id); // component id
    frame.push(msg_id as u8); // msg id low
    frame.push((msg_id >> 8) as u8);
    frame.push((msg_id >> 16) as u8);
    frame.extend_from_slice(payload);
    let crc = mav_crc16(&frame[1..], crc_extra); // 覆盖头[1..10] + payload
    frame.push(crc as u8);
    frame.push((crc >> 8) as u8);
    frame
}

/// 编码 COMMAND_LONG（标准顺序：target_sys, target_comp, command u16, confirmation,
/// param1-7 f32）。
pub fn encode_std_command_long(
    sys_id: u8,
    comp_id: u8,
    seq: u8,
    command: u16,
    params: [f32; 7],
    confirmation: u8,
) -> Vec<u8> {
    let mut p = [0u8; 33];
    p[0] = sys_id;
    p[1] = comp_id;
    p[2..4].copy_from_slice(&command.to_le_bytes());
    p[4] = confirmation;
    let put = |p: &mut [u8], off: usize, v: f32| p[off..off + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut p, 5, params[0]);
    put(&mut p, 9, params[1]);
    put(&mut p, 13, params[2]);
    put(&mut p, 17, params[3]);
    put(&mut p, 21, params[4]);
    put(&mut p, 25, params[5]);
    put(&mut p, 29, params[6]);
    encode_std_frame(76, sys_id, comp_id, seq, &p, 152)
}

/// 编码 PARAM_REQUEST_LIST（payload: target_sys, target_comp）。
pub fn encode_std_param_request_list(sys_id: u8, comp_id: u8, seq: u8) -> Vec<u8> {
    let mut p = [0u8; 2];
    p[0] = sys_id;
    p[1] = comp_id;
    encode_std_frame(21, sys_id, comp_id, seq, &p, 159)
}

/// 编码 PARAM_SET（标准顺序：param_id[16], param_value f32, param_type u8）。
pub fn encode_std_param_set(
    sys_id: u8,
    comp_id: u8,
    seq: u8,
    name: &str,
    value: f32,
) -> Vec<u8> {
    let mut p = [0u8; 21];
    let name_bytes = name.as_bytes();
    let n = name_bytes.len().min(15);
    p[0..n].copy_from_slice(&name_bytes[..n]);
    // p[n..16] 保持 0 即 NUL 结尾
    p[16..20].copy_from_slice(&value.to_le_bytes());
    p[20] = 9; // MAV_PARAM_TYPE_REAL32
    encode_std_frame(23, sys_id, comp_id, seq, &p, 168)
}

/// 编码 PARAM_REQUEST_READ（标准顺序：param_id[16], param_index i16）。
pub fn encode_std_param_request_read(
    sys_id: u8,
    comp_id: u8,
    seq: u8,
    name: &str,
    index: i16,
) -> Vec<u8> {
    let mut p = [0u8; 18];
    let name_bytes = name.as_bytes();
    let n = name_bytes.len().min(15);
    p[0..n].copy_from_slice(&name_bytes[..n]);
    p[16..18].copy_from_slice(&index.to_le_bytes());
    encode_std_frame(20, sys_id, comp_id, seq, &p, 214)
}


/// 增量解析器：喂入任意分片的字节流，吐出完整消息
///
/// MAVLink 帧最大 280 字节，内部缓冲不会无限增长。
pub struct MavlinkParser {
    buf: Vec<u8>,
    header: ::mavlink::MavHeader,
}

impl MavlinkParser {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(512),
            header: ::mavlink::MavHeader::default(),
        }
    }

    /// 喂入一批字节，返回本次可解析出的所有消息
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<(::mavlink::MavHeader, MavMessage)>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        // MAVLink v2 最小帧：magic(1)+len(1)+seq(1)+sysid(1)+compid(1)+msgid(1)
        //                      +incompat(1)+compat(1)+seq(2)+payload(0)+crc(2) = 12 字节
        const MIN_FRAME: usize = 12;
        loop {
            if self.buf.is_empty() {
                break;
            }
            // mavlink crate 从 Read 解析；用 Cursor 跟踪消费字节数
            let mut cursor = Cursor::new(self.buf.clone());
            let before = cursor.position() as usize;
            match ::mavlink::read_v2_msg::<MavMessage, _>(&mut cursor) {
                Ok((header, msg)) => {
                    let consumed = cursor.position() as usize - before;
                    if consumed == 0 {
                        // 防御：异常未消费，丢弃首字节避免死循环
                        self.buf.drain(..1);
                        continue;
                    }
                    self.header = header;
                    out.push((header, msg));
                    self.buf.drain(..consumed);
                }
                Err(_) => {
                    // mavlink 0.11.2 crate 的 PARAM_VALUE (msgid 22) 字段序列化顺序与
                    // 标准 common.xml 相反（crate 为 value->count->index->id->type，
                    // 标准为 id->value->type->count->index）。板子固件遵循标准顺序，
                    // 故 crate 解析必败（InvalidEnum）。这里对该 msg 用手写标准顺序
                    // 反序列化做兼容，其余消息仍走 crate 通用路径。
                    if let Some((header, msg, consumed)) = try_parse_param_value_std(&self.buf) {
                        self.header = header;
                        out.push((header, msg));
                        self.buf.drain(..consumed);
                        continue;
                    }
                    // 解析失败：区分「数据不足」与「非法帧起点」。
                    // 若剩余字节已足够容纳最小帧，说明当前 buf 开头不是合法帧
                    // （magic 不对或 CRC 错），必须丢弃首字节重新同步到下一个
                    // magic；否则保留缓冲等待更多字节到达。
                    if self.buf.len() >= MIN_FRAME {
                        self.buf.drain(..1);
                        // 防止缓冲无限增长（极端持续错位时兜底）
                        if self.buf.len() > 4096 {
                            self.buf.drain(..self.buf.len() - 4096);
                        }
                        continue;
                    }
                    break;
                }
            }
        }
        Ok(out)
    }

    pub fn last_header(&self) -> ::mavlink::MavHeader {
        self.header
    }
}

/// 标准 MAVLink v2 CRC（X.25/CRC-16-CCITT 反射，初值 0xFFFF，与 mavlink crate 一致）。
fn mav_crc16(buf: &[u8], extra: u8) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in buf {
        let mut tmp = (b ^ (crc as u8)) as u16;
        for _ in 0..8 {
            if tmp & 0x0001 != 0 {
                tmp >>= 1;
                tmp ^= 0xA001;
            } else {
                tmp >>= 1;
            }
        }
        crc = (crc >> 8) ^ tmp;
    }
    let mut tmp = (extra ^ (crc as u8)) as u16;
    for _ in 0..8 {
        if tmp & 0x0001 != 0 {
            tmp >>= 1;
            tmp ^= 0xA001;
        } else {
            tmp >>= 1;
        }
    }
    crc = (crc >> 8) ^ tmp;
    crc
}

/// 尝试用标准 common.xml 字段顺序解析 PARAM_VALUE (msgid 22)。
///
/// mavlink 0.11.2 crate 的 `PARAM_VALUE_DATA` 反序列化顺序（value/count/index/id/type）
/// 与标准（id/value/type/count/index）相反，导致板子（标准顺序）发出的帧被 crate 拒绝。
/// 这里手写标准顺序解析，校验 CRC（CRC_EXTRA=220）后构造正确的 `MavMessage`。
///
/// 返回 `(header, msg, consumed)`；若 buf 开头不是合法 PARAM_VALUE 帧则返回 None。
fn try_parse_param_value_std(
    buf: &[u8],
) -> Option<(::mavlink::MavHeader, MavMessage, usize)> {
    // 最小帧长：10 字节头 + 至少 1 字节 payload + 2 字节 CRC
    if buf.len() < 13 || buf[0] != 0xFD {
        return None;
    }
    let _incompat = buf[2];
    let _compat = buf[3];
    let seq = buf[4];
    let sys = buf[5];
    let comp = buf[6];
    let msg_id = (buf[7] as u32) | ((buf[8] as u32) << 8) | ((buf[9] as u32) << 16);
    if msg_id != 22 {
        return None;
    }
    let plen = buf[1] as usize;
    let frame_len = 10 + plen + 2;
    if buf.len() < frame_len {
        return None; // 数据尚不完整
    }
    let payload = &buf[10..10 + plen];
    if plen != 25 {
        return None;
    }
    // 校验 CRC（覆盖 头[1..10] + payload，附加 CRC_EXTRA=220）
    let crc_calc = mav_crc16(&buf[1..10 + plen], 220);
    let crc_wire = (buf[10 + plen] as u16) | ((buf[10 + plen + 1] as u16) << 8);
    if crc_calc != crc_wire {
        return None;
    }
    // 标准字段顺序：param_id[16] @0, param_value f32 @16, param_type u8 @20,
    //                param_count u16 @21, param_index u16 @23
    let mut id = [0u8; 16];
    id.copy_from_slice(&payload[0..16]);
    let value = f32::from_le_bytes([
        payload[16], payload[17], payload[18], payload[19],
    ]);
    let ptype = payload[20];
    let count = u16::from_le_bytes([payload[21], payload[22]]);
    let index = u16::from_le_bytes([payload[23], payload[24]]);
    let param_type = match ptype {
        1 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_UINT8,
        2 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_INT8,
        3 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_UINT16,
        4 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_INT16,
        5 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_UINT32,
        6 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_INT32,
        7 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_UINT64,
        8 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_INT64,
        9 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_REAL32,
        10 => ::mavlink::common::MavParamType::MAV_PARAM_TYPE_REAL64,
        _ => return None,
    };
    let header = ::mavlink::MavHeader {
        system_id: sys,
        component_id: comp,
        sequence: seq,
    };
    let data = ::mavlink::common::PARAM_VALUE_DATA {
        param_id: id,
        param_value: value,
        param_type,
        param_count: count,
        param_index: index,
    };
    let msg = MavMessage::PARAM_VALUE(data);
    Some((header, msg, frame_len))
}

/// 构造默认心跳头
pub fn default_header() -> ::mavlink::MavHeader {
    ::mavlink::MavHeader {
        system_id: 255,     // GCS 通常用 255
        component_id: 190,  // MAV_COMP_ID_MISSIONPLANNER
        sequence: 0,
    }
}

/// 心跳管理器：生成周期心跳字节帧
pub mod heartbeat {
    use super::*;

    /// 生成一条 GCS 心跳帧
    pub fn make_heartbeat() -> Vec<u8> {
        let msg = MavMessage::HEARTBEAT(::mavlink::common::HEARTBEAT_DATA {
            custom_mode: 0,
            mavtype: ::mavlink::common::MavType::MAV_TYPE_GCS,
            autopilot: ::mavlink::common::MavAutopilot::MAV_AUTOPILOT_INVALID,
            base_mode: ::mavlink::common::MavModeFlag::empty(),
            system_status: ::mavlink::common::MavState::MAV_STATE_ACTIVE,
            mavlink_version: 3,
        });
        encode_v2(&default_header(), &msg).unwrap_or_default()
    }
}
