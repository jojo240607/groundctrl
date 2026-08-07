//! 飞行日志：tlog 记录与回放
//!
//! `LogManager` 把经过总线的 MAVLink 帧以「tlog」格式记录（时间戳 + 原始 v2 字节），
//! 支持：
//! - 实时记录（`record`）
//! - 回放驱动（`replay`）：逐帧解码并通过回调把消息交回上层
//! - 导出 CSV（遥测快照）与 KML（航迹）
//!
//! tlog 帧格式（与 pymavlink 兼容思路，简化）：
//! `[u64 le: timestamp_ms][u32 le: len][len bytes: mavlink v2 frame]`

use std::io::{Read, Write};

use crate::mlink;

/// 一条 tlog 记录
#[derive(Debug, Clone)]
pub struct LogFrame {
    pub ts_ms: u64,
    pub bytes: Vec<u8>,
}

/// 日志管理器（内存记录 + 可选落盘）
#[derive(Debug, Default)]
pub struct LogManager {
    frames: Vec<LogFrame>,
    recording: bool,
}

impl LogManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 开始/停止记录
    pub fn set_recording(&mut self, on: bool) {
        self.recording = on;
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// 记录一帧（仅当 recording=true 时入队）
    pub fn record(&mut self, ts_ms: u64, bytes: &[u8]) {
        if self.recording {
            self.frames.push(LogFrame {
                ts_ms,
                bytes: bytes.to_vec(),
            });
        }
    }

    /// 记录一条已编码好的帧
    pub fn record_msg(&mut self, ts_ms: u64, header: &::mavlink::MavHeader, msg: &mlink::MavMessage) {
        if let Ok(b) = mlink::encode_v2(header, msg) {
            self.record(ts_ms, &b);
        }
    }

    /// 帧数
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// 回放：逐帧解码并调用 cb（按时间戳顺序）
    pub fn replay<F>(&self, mut cb: F) -> crate::error::Result<()>
    where
        F: FnMut(u64, ::mavlink::MavHeader, mlink::MavMessage),
    {
        for f in &self.frames {
            let mut cur = std::io::Cursor::new(&f.bytes);
            if let Ok((header, msg)) =
                ::mavlink::read_v2_msg::<mlink::MavMessage, _>(&mut cur)
            {
                cb(f.ts_ms, header, msg);
            }
        }
        Ok(())
    }

    /// 把内存日志序列化为 tlog 字节流
    pub fn to_tlog(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for f in &self.frames {
            out.extend_from_slice(&f.ts_ms.to_le_bytes());
            out.extend_from_slice(&(f.bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(&f.bytes);
        }
        out
    }

    /// 从 tlog 字节流载入
    pub fn from_tlog(mut data: &[u8]) -> crate::error::Result<Self> {
        let mut frames = Vec::new();
        let mut cur = std::io::Cursor::new(&mut data);
        loop {
            let mut ts_buf = [0u8; 8];
            let mut len_buf = [0u8; 4];
            if cur.read_exact(&mut ts_buf).is_err() {
                break;
            }
            if cur.read_exact(&mut len_buf).is_err() {
                break;
            }
            let ts_ms = u64::from_le_bytes(ts_buf);
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut bytes = vec![0u8; len];
            if cur.read_exact(&mut bytes).is_err() {
                break;
            }
            frames.push(LogFrame { ts_ms, bytes });
        }
        Ok(Self {
            frames,
            recording: false,
        })
    }

    /// 写入 tlog 文件
    pub fn save_file(&self, path: &str) -> std::io::Result<()> {
        let mut f = std::fs::File::create(path)?;
        f.write_all(&self.to_tlog())?;
        Ok(())
    }

    /// 从 tlog 文件读取
    pub fn load_file(path: &str) -> std::io::Result<Self> {
        let data = std::fs::read(path)?;
        Self::from_tlog(&data).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}
