//! MAVLink 服务层封装
//!
//! 选用 `common` dialect 作为默认消息类型（覆盖绝大部分飞控）。
//! 若需要厂商扩展（ardupilotmega），可在路由层按 system_id 区分。

use std::io::Cursor;

use crate::error::Result;

/// 项目统一消息类型（common dialect 扁平枚举，mavlink 0.11 为具体类型）
pub type MavMessage = ::mavlink::common::MavMessage;

/// 把一条消息编码为 MAVLink v2 字节帧
pub fn encode_v2(header: &::mavlink::MavHeader, msg: &MavMessage) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(280);
    ::mavlink::write_v2_msg(&mut buf, *header, msg)
        .map(|_| buf)
        .map_err(|e| crate::error::GcError::Mavlink(e.to_string()))
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
