//! UDP 链路（局域网 / 地面 WiFi 数传）

use async_trait::async_trait;
use tokio::net::UdpSocket;

use crate::error::{GcError, Result};
use crate::link::{Link, LinkQuality, LinkStats};

pub struct UdpConfig {
    pub bind_addr: String,  // 本地绑定，如 "0.0.0.0:14550"
    pub target_addr: String, // 远端飞控，如 "127.0.0.1:14551"
}

pub struct UdpLink {
    socket: UdpSocket,
    target: String,
    stats: LinkStats,
    open: bool,
    name: String,
}

impl UdpLink {
    pub async fn new(cfg: UdpConfig) -> Result<Self> {
        let socket = UdpSocket::bind(&cfg.bind_addr).await?;
        socket.connect(&cfg.target_addr).await?;
        let name = format!("udp {}->{}", cfg.bind_addr, cfg.target_addr);
        Ok(Self {
            socket,
            target: cfg.target_addr,
            stats: LinkStats::new(),
            open: true,
            name,
        })
    }
}

#[async_trait]
impl Link for UdpLink {
    async fn send(&self, bytes: &[u8]) -> Result<()> {
        self.stats.add_sent(bytes.len() as u64).await;
        self.socket
            .send(bytes)
            .await
            .map(|_| ())
            .map_err(|e| GcError::Link(e.to_string()))
    }

    async fn recv(&self) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; 4096];
        let n = self
            .socket
            .recv(&mut buf)
            .await
            .map_err(|e| GcError::Link(e.to_string()))?;
        buf.truncate(n);
        self.stats.add_recv(n as u64).await;
        Ok(buf)
    }

    fn quality(&self) -> LinkQuality {
        LinkQuality::default()
    }

    fn is_open(&self) -> bool {
        self.open
    }

    fn name(&self) -> String {
        self.name.clone()
    }
}
