//! 串口链路（桌面：tokio-serial）
//!
//! 移动端（Android USB-OTG / iOS）不在此实现，由平台壳经 FFI 提供链路口。

use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_serial::SerialStream;

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
            let builder = tokio_serial::new(&port, baud)
                .dtr_on_open(false); // DTR=true 会触发 Windows 复位 USB CDC 设备导致下行断；飞控 bulk-IN 不依赖 DTR
            let stream = match SerialStream::open(&builder) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("serial open {port} failed: {e}");
                    return;
                }
            };
            open_flag.store(true, std::sync::atomic::Ordering::SeqCst);

            // 用 tokio::io::split 把同一底层句柄拆成独立的读半/写半。
            // 关键：不能用共享 Mutex 包住单个 stream 再让 reader 跨 await 持有锁，
            // 那样 writer 永远拿不到锁 -> 上行写入被饿死。split 后读/写各自无锁，
            // 底层 Windows 串口允许读/写 overlapped 操作并发（USB CDC 端点独立）。
            let (mut reader_half, mut writer_half) = tokio::io::split(stream);

            // 读任务：从串口读取字节，投递到 tx_bytes 供 recv() 消费
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    match tokio::io::AsyncReadExt::read(&mut reader_half, &mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
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
                            match tokio::io::AsyncWriteExt::write(&mut writer_half, &data[off..]).await {
                                Ok(0) => break,
                                Err(e) => {
                                    tracing::error!("[serial] write err: {e}");
                                    break;
                                }
                                Ok(n) => {
                                    off += n;
                                }
                            }
                        }
                        match tokio::io::AsyncWriteExt::flush(&mut writer_half).await {
                            Ok(()) => {}
                            Err(e) => tracing::error!("[serial] flush err: {e}"),
                        }
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
        // 超时：串口/CDC 长时间无数据时返回 WouldBlock，让调用方（main 循环）能
        // 周期性地检查自己的 deadline 并正常退出。否则 rx.recv() 永久阻塞，
        // main 会卡死在 recv() 上、永远到不了 deadline 检查。
        match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
            Ok(Some(b)) => {
                self.stats.add_recv(b.len() as u64).await;
                Ok(b)
            }
            Ok(None) => Err(GcError::Link("serial closed".into())),
            Err(_) => Err(GcError::Link("recv timeout".into())),
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
