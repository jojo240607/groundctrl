//! MAVLink 服务层封装
//!
//! 统一基于共用 crate `mavlink-core`（标准 common.xml 字段顺序，与固件/仿真器共用）。
//! 迁移说明：原先针对官方 `mavlink` crate 0.11.2 字段序列化顺序差异而手写的
//! `try_parse_heartbeat_std` / `try_parse_param_value_std` 兼容垫片已删除——
//! `mavlink_core::common::read_v2_msg` 按标准顺序解析，字节级与板端一致；
//! `encode_std_*` 保留为兼容签名，内部基于 `mavlink_core::common::build_v2`。

use crate::error::Result;

/// 项目统一消息类型 + 帧头（来自共用 crate，命名与官方 `mavlink` crate 对齐）
pub use mavlink_core::common::{MavHeader, MavMessage, read_v2_msg, write_v2_msg};

/// 手写编码的厂商扩展消息（围栏 FENCE_POINT / FENCE_FETCH_POINT）
pub mod fence;

/// 手写编码的 MAVLink FTP（FILE_TRANSFER_PROTOCOL, msg 110，固件升级）
pub mod ftp;

/// 把一条消息编码为 MAVLink v2 字节帧（标准字段顺序）
pub fn encode_v2(header: &MavHeader, msg: &MavMessage) -> Result<Vec<u8>> {
    mavlink_core::common::write_v2_msg(header, msg)
        .map_err(|e| crate::error::GcError::Mavlink(e))
}

// ============================================================================
// 标准 MAVLink v2 帧编码（兼容签名）
//
// 历史背景：官方 `mavlink` crate 0.11.2 的 `common` dialect 中部分消息（如
// COMMAND_LONG / PARAM_VALUE）字段序列化顺序与标准 common.xml 相反，导致上行
// 指令被板端拒绝，故地面站原先手写标准顺序编码。迁移至 `mavlink-core` 后其
// `write_v2_msg` 已按标准顺序编码；但以下按原始 u16 命令字组帧的入口仍被
// telemetry_hub / tools / 诊断示例使用，故保留签名，统一走 `build_v2`
// （字节级标准一致，CRC_EXTRA 取自标准表）。
// ============================================================================

/// 编码一个标准 MAVLink v2 帧（10 字节头 + payload + 2 字节 CRC）。
/// `crc_extra` 为兼容参数（`build_v2` 使用标准 CRC_EXTRA 表，二者一致）。
pub fn encode_std_frame(
    msg_id: u32,
    sys_id: u8,
    comp_id: u8,
    seq: u8,
    payload: &[u8],
    _crc_extra: u8,
) -> Vec<u8> {
    mavlink_core::common::build_v2(msg_id, sys_id, comp_id, seq, payload)
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
    header: MavHeader,
}

impl MavlinkParser {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(512),
            header: MavHeader::default(),
        }
    }

    /// 喂入一批字节，返回本次可解析出的所有消息
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<(MavHeader, MavMessage)>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        // MAVLink v2 最小帧：magic(1)+len(1)+seq(1)+sysid(1)+compid(1)+msgid(1)
        //                      +incompat(1)+compat(1)+seq(2)+payload(0)+crc(2) = 12 字节
        const MIN_FRAME: usize = 12;
        loop {
            if self.buf.is_empty() {
                break;
            }
            // `read_v2_msg` 自动扫描下一处合法 magic；返回 (header, msg, 消耗字节数)。
            match mavlink_core::common::read_v2_msg(&self.buf) {
                Some((header, msg, consumed)) => {
                    if consumed == 0 {
                        // 防御：异常未消费，丢弃首字节避免死循环
                        self.buf.drain(..1);
                        continue;
                    }
                    self.header = header;
                    out.push((header, msg));
                    self.buf.drain(..consumed);
                }
                None => {
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

    pub fn last_header(&self) -> MavHeader {
        self.header
    }
}

/// 构造默认心跳头
pub fn default_header() -> MavHeader {
    MavHeader {
        system_id: 255,    // GCS 通常用 255
        component_id: 190, // MAV_COMP_ID_MISSIONPLANNER
        sequence: 0,
    }
}

/// 心跳管理器：生成周期心跳字节帧
pub mod heartbeat {
    use mavlink_core::common::{MavAutopilot, MavModeFlag, MavState, MavType};

    use super::*;

    /// 生成一条 GCS 心跳帧
    pub fn make_heartbeat() -> Vec<u8> {
        let msg = MavMessage::HEARTBEAT(mavlink_core::common::HEARTBEAT_DATA {
            custom_mode: 0,
            mavtype: MavType::MAV_TYPE_GCS,
            autopilot: MavAutopilot::MAV_AUTOPILOT_INVALID,
            base_mode: MavModeFlag::empty(),
            system_status: MavState::MAV_STATE_ACTIVE,
            mavlink_version: 3,
        });
        encode_v2(&default_header(), &msg).unwrap_or_default()
    }
}
