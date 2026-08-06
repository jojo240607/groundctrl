//! 串口链路（桌面：tokio-serial）
//!
//! 移动端（Android USB-OTG / iOS）不在此实现，由平台壳经 FFI 提供链路口。

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tokio_serial::{SerialStream, SerialPortBuilderExt};

use crate::error::{GcError, Result};
use crate::link::{Link, LinkQuality, LinkStats};

#[derive(Debug, Clone)]
pub struct SerialConfig {
    pub port: String,
    pub baud_rate: u32,
}

pub struct SerialLink {
    cfg: SerialConfig,
    stats: LinkStats,
    open: Arc<AtomicBool>,
    /// 发送端（写线程接收命令）
    tx: mpsc::UnboundedSender<Vec<u8>>,
    /// 接收端（读线程投喂字节）
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
}

impl SerialLink {
    pub fn new(cfg: SerialConfig) -> Self {
        let (tx, rx_cmd) = mpsc::unbounded_channel::<Vec<u8>>();
        let (tx_bytes, rx_bytes) = mpsc::unbounded_channel::<Vec<u8>>();
        let open = Arc::new(AtomicBool::new(false));

        // 后台任务：持有真正的串口，桥接 tx/rx 通道
        let port = cfg.port.clone();
        let baud = cfg.baud_rate;
        let open_flag = open.clone();
        tokio::spawn(async move {
            let builder = tokio_serial::new(&port, baud);
            let stream = match SerialStream::open(&builder) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("serial open {port} failed: {e}");
                    return;
                }
            };
            open_flag.store(true, std::sync::atomic::Ordering::SeqCst);

            let stream = std::sync::Arc::new(tokio::sync::Mutex::new(stream));
            let reader_stream = stream.clone();
            let writer_stream = stream.clone();

            // 读任务：从串口读取字节，投递到 tx_bytes 供 recv() 消费
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    let mut s = reader_stream.lock().await;
                    match tokio::io::AsyncReadExt::read(&mut *s, &mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            drop(s);
                            if tx_bytes.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            });

            // 命令 -> 写串口
            let mut rx_cmd = rx_cmd;
            loop {
                match rx_cmd.recv().await {
                    None => break,
                    Some(data) => {
                        let mut off = 0;
                        while off < data.len() {
                            let mut s = writer_stream.lock().await;
                            match tokio::io::AsyncWriteExt::write(&mut *s, &data[off..]).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    off += n;
                                }
                            }
                        }
                        let mut s = writer_stream.lock().await;
                        let _ = tokio::io::AsyncWriteExt::flush(&mut *s).await;
                    }
                }
            }
        });

        Self {
            cfg,
            stats: LinkStats::new(),
            open,
            tx,
            rx: tokio::sync::Mutex::new(rx_bytes),
        }
    }
}

#[async_trait]
impl Link for SerialLink {
    async fn send(&self, bytes: &[u8]) -> Result<()> {
        self.stats.add_sent(bytes.len() as u64).await;
        self.tx
            .send(bytes.to_vec())
            .map_err(|_| GcError::Link("serial task gone".into()))
    }

    async fn recv(&self) -> Result<Vec<u8>> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(b) => {
                self.stats.add_recv(b.len() as u64).await;
                Ok(b)
            }
            None => Err(GcError::Link("serial closed".into())),
        }
    }

    fn quality(&self) -> LinkQuality {
        // 同步快照：用一个阻塞运行时无关的近似（tokio 单线程下安全）
        // 这里简化为默认，真实 RSSI 由介质提供
        LinkQuality::default()
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    fn name(&self) -> String {
        format!("serial {}@{}", self.cfg.port, self.cfg.baud_rate)
    }
}
