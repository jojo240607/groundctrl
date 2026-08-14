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

/// 从日志提取的一条时间序列（供图表分析）
#[derive(Debug, Clone)]
pub struct LogSeries {
    /// 字段名（如「相对高度」）
    pub name: String,
    /// 单位（如 m / m/s / % / deg）
    pub unit: String,
    /// 采样点 (相对首帧秒, 值)
    pub points: Vec<(f64, f64)>,
}

impl LogSeries {
    fn new(name: &str, unit: &str) -> Self {
        Self {
            name: name.into(),
            unit: unit.into(),
            points: Vec::new(),
        }
    }
}

/// 日志管理器（内存记录 + 可选落盘）
#[derive(Debug, Default, Clone)]
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

    /// 解析日志帧，提取常用遥测字段的时间序列（供图表分析）。
    ///
    /// 时间轴为相对首帧的秒数；无数据字段返回空序列。
    pub fn analyze(&self) -> Vec<LogSeries> {
        let mut series: Vec<LogSeries> = vec![
            LogSeries::new("相对高度", "m"),
            LogSeries::new("绝对高度", "m"),
            LogSeries::new("地速", "m/s"),
            LogSeries::new("空速", "m/s"),
            LogSeries::new("爬升率", "m/s"),
            LogSeries::new("油门", "%"),
            LogSeries::new("电量", "%"),
            LogSeries::new("电压", "V"),
            LogSeries::new("横滚", "deg"),
            LogSeries::new("俯仰", "deg"),
            LogSeries::new("偏航", "deg"),
            LogSeries::new("HDOP", "m"),
            LogSeries::new("卫星数", ""),
        ];
        let t0 = self.frames.first().map(|f| f.ts_ms).unwrap_or(0);
        let mut first_seen = [false; 13];

        let push = |series: &mut Vec<LogSeries>, idx: usize, first: &mut [bool; 13], t: f64, v: f64| {
            first[idx] = true;
            series[idx].points.push((t, v));
        };

        for f in &self.frames {
            let t = (f.ts_ms.saturating_sub(t0)) as f64 / 1000.0;
            let mut cur = std::io::Cursor::new(&f.bytes);
            let Ok((_h, msg)) = ::mavlink::read_v2_msg::<mlink::MavMessage, _>(&mut cur) else {
                continue;
            };
            match msg {
                mlink::MavMessage::GLOBAL_POSITION_INT(d) => {
                    push(&mut series, 0, &mut first_seen, t, d.relative_alt as f64 / 1000.0);
                    push(&mut series, 1, &mut first_seen, t, d.alt as f64 / 1000.0);
                }
                mlink::MavMessage::VFR_HUD(d) => {
                    push(&mut series, 2, &mut first_seen, t, d.groundspeed as f64);
                    push(&mut series, 3, &mut first_seen, t, d.airspeed as f64);
                    push(&mut series, 4, &mut first_seen, t, d.climb as f64);
                    push(&mut series, 5, &mut first_seen, t, d.throttle as f64);
                }
                mlink::MavMessage::SYS_STATUS(d) => {
                    push(&mut series, 6, &mut first_seen, t, d.battery_remaining as f64);
                    push(&mut series, 7, &mut first_seen, t, d.voltage_battery as f64 / 1000.0);
                }
                mlink::MavMessage::ATTITUDE(d) => {
                    let r2d = 180.0_f64 / std::f64::consts::PI;
                    push(&mut series, 8, &mut first_seen, t, d.roll as f64 * r2d);
                    push(&mut series, 9, &mut first_seen, t, d.pitch as f64 * r2d);
                    push(&mut series, 10, &mut first_seen, t, d.yaw as f64 * r2d);
                }
                mlink::MavMessage::GPS_RAW_INT(d) => {
                    if d.eph != 65535 {
                        push(&mut series, 11, &mut first_seen, t, d.eph as f64 / 100.0);
                    }
                    push(&mut series, 12, &mut first_seen, t, d.satellites_visible as f64);
                }
                _ => {}
            }
        }

        series.retain(|s| !s.points.is_empty());
        series
    }
}
