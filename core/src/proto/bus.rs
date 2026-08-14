//! 内部消息总线
//!
//! 基于 tokio broadcast：链路层解析出的消息发布于此，UI 与逻辑订阅。
//! 同时提供 VehicleModel 的状态广播，供 UI 直接订阅最新遥测快照。

use tokio::sync::broadcast;

use crate::mlink;

/// 总线中流转的高层事件
#[derive(Debug, Clone)]
pub enum BusEvent {
    /// 收到一帧 MAVLink 消息（含来源链路名）
    Mavlink {
        link: String,
        header: ::mavlink::MavHeader,
        msg: mlink::MavMessage,
    },
    /// 链路状态变化
    LinkState { link: String, open: bool },
    /// 参数缓存变化（拉取进度或新值）
    Params {
        link: String,
        complete: bool,
        received: u16,
        expected: u16,
        entries: Vec<crate::vehicle::params::ParamEntry>,
    },
    /// 告警事件（每次新触发的告警）
    Alarm { link: String, alarm: crate::services::alarms::Alarm },
    /// 日志：一帧已记录的 tlog 字节
    LogFrame { link: String, ts_ms: u64, bytes: Vec<u8> },
    /// 收到一帧 FENCE_POINT（厂商扩展消息，不进 Mavlink 事件；坐标单位 1e7 度）
    FencePoint {
        link: String,
        header: ::mavlink::MavHeader,
        idx: u8,
        count: u8,
        lat: i32,
        lng: i32,
    },
    /// 收到一帧 MAVLink FTP（FILE_TRANSFER_PROTOCOL，手写编解码，不进 Mavlink 事件）
    Ftp {
        link: String,
        header: ::mavlink::MavHeader,
        payload: crate::mlink::ftp::FtpPayload,
    },
}

/// 消息总线封装
#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<BusEvent>,
}

impl Bus {
    pub fn new(cap: usize) -> Self {
        let (tx, _rx) = broadcast::channel(cap);
        Self { tx }
    }

    pub fn publish(&self, ev: BusEvent) {
        // 无订阅者时忽略错误
        let _ = self.tx.send(ev);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BusEvent> {
        self.tx.subscribe()
    }
}

/// 遥测快照广播：UI 订阅此通道获取最新 VehicleModel
pub type TelemetryBus = broadcast::Sender<crate::vehicle::VehicleModel>;
