//! MAVLink FTP（FILE_TRANSFER_PROTOCOL, msg 110）手写编解码
//!
//! MAVLink 0.11.2 的 `common` dialect 不含该消息（协议按“应用层服务”独立维护），
//! 因此按 MAVLink v2 协议手写编解码，与 `fence.rs` 同模式：
//! - 消息 110 `FILE_TRANSFER_PROTOCOL`，crc_extra 84（取自 pymavlink 2.4.43 生成物）
//! - 载荷布局：`<BBB251B` = target_network(1) + target_system(1) + target_component(1)
//!   + payload(251)，消息总长 254 字节
//! - FTP 协议体（251 字节中的前 250 字节）：
//!   seq_number(u16) session(u8) opcode(u8) size(u8) req_opcode(u8)
//!   burst_complete(u8) padding(u8) offset(u64) data(234)
//!
//! 协议细节见 https://mavlink.io/en/services/ftp.html

use crate::error::{GcError, Result};

pub const FTP_MSG_ID: u32 = 110;
const FTP_CRC_EXTRA: u8 = 84;
/// FTP 协议体 data 区最大长度
pub const DATA_MAX: usize = 234;
/// 单次 WriteFile 最大负载（data 前 8 字节为 offset，剩余 226 字节为内容）
pub const WRITE_CHUNK_MAX: usize = DATA_MAX - 8;

// ---- opcode ----
pub const OP_NONE: u8 = 0;
pub const OP_TERMINATE_SESSION: u8 = 1;
pub const OP_RESET_SESSIONS: u8 = 2;
pub const OP_LIST_DIRECTORY: u8 = 3;
pub const OP_OPEN_FILE_RO: u8 = 4;
pub const OP_READ_FILE: u8 = 5;
pub const OP_CREATE_FILE: u8 = 6;
pub const OP_WRITE_FILE: u8 = 7;
pub const OP_REMOVE_FILE: u8 = 8;
pub const OP_RENAME: u8 = 9;
pub const OP_CALC_FILE_CRC32: u8 = 10;
pub const OP_BURST_READ_FILE: u8 = 11;
pub const OP_ACK: u8 = 128;
pub const OP_NACK: u8 = 129;

// ---- 错误码（NACK 的 data[0]）----
pub const ERR_NONE: u8 = 0;
pub const ERR_FAIL: u8 = 1;
pub const ERR_FAILERRNO: u8 = 2;
pub const ERR_INVALIDDATASIZE: u8 = 3;
pub const ERR_INVALIDSESSION: u8 = 4;
pub const ERR_NOSESSIONSAVAILABLE: u8 = 5;
pub const ERR_EOF: u8 = 6;
pub const ERR_UNKNOWNCOMMAND: u8 = 7;
pub const ERR_FILEEXISTS: u8 = 8;
pub const ERR_FILEPROTECTED: u8 = 9;
pub const ERR_FILENOTFOUND: u8 = 10;

// ---- ListDirectory 条目类型 ----
pub const FILETYPE_FILE: u8 = 1;
pub const FILETYPE_DIRECTORY: u8 = 2;
pub const FILETYPE_SKIP: u8 = 3;

/// 解析出的 MAVLink FTP 请求/应答
#[derive(Debug, Clone, PartialEq)]
pub struct FtpPayload {
    pub seq: u16,
    pub session: u8,
    pub opcode: u8,
    /// data 有效长度
    pub size: u8,
    /// 应答中回填的原始请求 opcode（请求侧为 0）
    pub req_opcode: u8,
    pub burst: u8,
    /// 读写偏移
    pub offset: u64,
    /// 有效数据（最多 234 字节）
    pub data: Vec<u8>,
}

impl FtpPayload {
    /// 构造一个请求（size 由 data 长度推出）
    pub fn request(seq: u16, session: u8, opcode: u8, data: Vec<u8>) -> Self {
        let size = data.len().min(DATA_MAX) as u8;
        Self {
            seq,
            session,
            opcode,
            size,
            req_opcode: 0,
            burst: 0,
            offset: 0,
            data: data.into_iter().take(DATA_MAX).collect(),
        }
    }

    /// 构造 ACK（回填请求 opcode）
    pub fn ack(seq: u16, session: u8, req_opcode: u8, offset: u64, data: Vec<u8>) -> Self {
        let size = data.len().min(DATA_MAX) as u8;
        Self {
            seq,
            session,
            opcode: OP_ACK,
            size,
            req_opcode,
            burst: 0,
            offset,
            data: data.into_iter().take(DATA_MAX).collect(),
        }
    }

    /// 构造 NACK（data[0] = 错误码）
    pub fn nack(seq: u16, session: u8, req_opcode: u8, err: u8) -> Self {
        Self {
            seq,
            session,
            opcode: OP_NACK,
            size: 1,
            req_opcode,
            burst: 0,
            offset: 0,
            data: vec![err],
        }
    }

    /// NACK 错误码
    pub fn err(&self) -> u8 {
        self.data.first().copied().unwrap_or(ERR_FAIL)
    }

    /// 是否为 ACK
    pub fn is_ack(&self) -> bool {
        self.opcode == OP_ACK
    }

    /// 是否为 NACK
    pub fn is_nack(&self) -> bool {
        self.opcode == OP_NACK
    }
}

/// 编码一条 MAV_FTP 帧（v2，payload 254 字节，crc_extra 84）
pub fn encode_ftp(
    header: &::mavlink::MavHeader,
    target_system: u8,
    target_component: u8,
    p: &FtpPayload,
) -> Result<Vec<u8>> {
    if p.data.len() > DATA_MAX {
        return Err(GcError::Mavlink("FTP 数据超长".into()));
    }
    let mut body = [0u8; 251];
    body[0..2].copy_from_slice(&p.seq.to_le_bytes());
    body[2] = p.session;
    body[3] = p.opcode;
    body[4] = p.size;
    body[5] = p.req_opcode;
    body[6] = p.burst;
    // body[7] padding = 0
    body[8..16].copy_from_slice(&p.offset.to_le_bytes());
    body[16..16 + p.data.len()].copy_from_slice(&p.data);

    let mut payload = Vec::with_capacity(254);
    payload.push(0); // target_network
    payload.push(target_system);
    payload.push(target_component);
    payload.extend_from_slice(&body);

    let mut buf = Vec::with_capacity(10 + 254 + 2);
    buf.push(0xFD); // MAV_STX_V2
    buf.push(254);
    buf.push(0); // incompat_flags
    buf.push(0); // compat_flags
    buf.push(header.sequence);
    buf.push(header.system_id);
    buf.push(header.component_id);
    buf.extend_from_slice(&FTP_MSG_ID.to_le_bytes()[..3]);
    buf.extend_from_slice(&payload);
    let mut crc_input = Vec::with_capacity(9 + 254 + 1);
    crc_input.extend_from_slice(&buf[1..]);
    crc_input.push(FTP_CRC_EXTRA);
    let crc = crate::mlink::fence::crc16_mcrf4cc(&crc_input);
    buf.extend_from_slice(&crc.to_le_bytes());
    Ok(buf)
}

/// 解析一条 MAV_FTP 帧，返回 (target_system, target_component, payload)
pub fn decode_ftp(bytes: &[u8]) -> Option<(u8, u8, FtpPayload)> {
    if bytes.len() < 12 || bytes[0] != 0xFD {
        return None;
    }
    let payload_len = bytes[1] as usize;
    let total = 10 + payload_len + 2;
    if bytes.len() < total || payload_len != 254 {
        return None;
    }
    let msg_id = u32::from_le_bytes([bytes[7], bytes[8], bytes[9], 0]);
    if msg_id != FTP_MSG_ID {
        return None;
    }
    let mut crc_input = Vec::with_capacity(9 + payload_len + 1);
    crc_input.extend_from_slice(&bytes[1..10 + payload_len]);
    crc_input.push(FTP_CRC_EXTRA);
    if crate::mlink::fence::crc16_mcrf4cc(&crc_input)
        != u16::from_le_bytes([bytes[total - 2], bytes[total - 1]])
    {
        return None;
    }
    let target_system = bytes[10 + 1];
    let target_component = bytes[10 + 2];
    let b = &bytes[10 + 3..10 + 254];
    let size = (b[4] as usize).min(DATA_MAX);
    Some((
        target_system,
        target_component,
        FtpPayload {
            seq: u16::from_le_bytes([b[0], b[1]]),
            session: b[2],
            opcode: b[3],
            size: size as u8,
            req_opcode: b[5],
            burst: b[6],
            offset: u64::from_le_bytes(b[8..16].try_into().ok()?),
            data: b[16..16 + size].to_vec(),
        },
    ))
}

/// 标准 CRC32（IEEE，用于 CalcFileCRC32 校验）
pub fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 编码 -> 解码往返一致（含 CRC 校验）
    #[test]
    fn ftp_roundtrip() {
        let header = ::mavlink::MavHeader {
            system_id: 255,
            component_id: 190,
            sequence: 9,
        };
        let req = FtpPayload::request(7, 3, OP_CREATE_FILE, b"firmware.bin".to_vec());
        let bytes = encode_ftp(&header, 1, 1, &req).unwrap();
        assert_eq!(bytes.len(), 10 + 254 + 2);
        let (tsys, tcomp, got) = decode_ftp(&bytes).expect("parse back");
        assert_eq!((tsys, tcomp), (1, 1));
        assert_eq!(got.seq, 7);
        assert_eq!(got.session, 3);
        assert_eq!(got.opcode, OP_CREATE_FILE);
        assert_eq!(got.size, 12);
        assert_eq!(got.data, b"firmware.bin");
    }

    /// 篡改一字节后应无法解析（CRC 校验生效）
    #[test]
    fn ftp_rejects_tampered() {
        let header = ::mavlink::MavHeader {
            system_id: 255,
            component_id: 190,
            sequence: 1,
        };
        let req = FtpPayload::request(1, 0, OP_READ_FILE, Vec::new());
        let mut bytes = encode_ftp(&header, 1, 1, &req).unwrap();
        bytes[20] ^= 0xFF;
        assert!(decode_ftp(&bytes).is_none());
    }

    /// WriteFile 负载布局：offset(8) + 内容
    #[test]
    fn write_payload_layout() {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&1234u64.to_le_bytes());
        chunk.extend_from_slice(&[0xAA; 100]);
        let req = FtpPayload::request(2, 1, OP_WRITE_FILE, chunk.clone());
        assert_eq!(req.size, 108);
        let header = ::mavlink::MavHeader {
            system_id: 255,
            component_id: 190,
            sequence: 2,
        };
        let bytes = encode_ftp(&header, 1, 1, &req).unwrap();
        let (_, _, got) = decode_ftp(&bytes).unwrap();
        // WriteFile 的 offset 位于 data[0..8]，header offset 字段为 0
        assert_eq!(got.offset, 0);
        assert_eq!(
            u64::from_le_bytes(got.data[0..8].try_into().unwrap()),
            1234
        );
        assert_eq!(got.data.len(), 108);
        assert_eq!(got.data[8..], [0xAA; 100]);
    }

    /// CRC32 已知向量校验
    #[test]
    fn crc32_known_value() {
        assert_eq!(crc32_ieee(b""), 0);
        assert_eq!(crc32_ieee(b"123456789"), 0xCBF4_3926);
    }
}
