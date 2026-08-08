//! 链路抽象层
//!
//! `Link` trait 定义统一的收发接口；各平台/介质提供具体实现。
//! 字节流由上层 MAVLink 解析器处理，链路层不感知协议内容。

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod serial;
pub mod udp;
pub mod sim;

/// 链路质量统计
#[derive(Debug, Clone, Default)]
pub struct LinkQuality {
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub packets_recv: u64,
    /// 0..100，由实现方按介质估算（如 RSSI）
    pub signal_pct: u8,
}

/// 统一链路接口
///
/// 实现需保证 `Send + Sync`，可在 tokio 多任务间共享。
#[async_trait]
pub trait Link: Send + Sync {
    /// 发送原始字节
    async fn send(&self, bytes: &[u8]) -> Result<()>;

    /// 接收一批原始字节（阻塞直到有数据或链路关闭）
    async fn recv(&self) -> Result<Vec<u8>>;

    /// 当前链路质量
    fn quality(&self) -> LinkQuality;

    /// 是否已连接/打开
    fn is_open(&self) -> bool;

    /// 友好名称（用于 UI 显示）
    fn name(&self) -> String;
}

pub type LinkHandle = Arc<dyn Link>;

/// 便捷：把任意 Link 包进 Arc 供共享
pub fn share(link: impl Link + 'static) -> LinkHandle {
    Arc::new(link)
}

// 内部：带质量计数器的链路包装，方便实现方累加统计
pub(crate) struct LinkStats {
    inner: Mutex<LinkQuality>,
}

impl LinkStats {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LinkQuality::default()),
        }
    }

    pub async fn add_sent(&self, n: u64) {
        let mut q = self.inner.lock().await;
        q.bytes_sent += n;
    }

    pub async fn add_recv(&self, n: u64) {
        let mut q = self.inner.lock().await;
        q.bytes_recv += n;
        q.packets_recv += 1;
    }
}

use crate::error::Result;
