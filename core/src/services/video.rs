//! 视频接收服务（P3-2）：UDP MJPEG 流接收。
//!
//! 图传/摄像头端把每帧 JPEG 编码为一个（或连续多个）UDP 数据报：
//! - 单帧单包：数据报以 FFD8 开头、FFD9 结尾；
//! - 单帧分包：跨数据报传输，按 SOI(FFD8)/EOI(FFD9) 重组；
//! - 单包多帧：同一数据报含多帧时逐个提取。
//!
//! 接收线程只做「分帧 + 保留最新一帧」，解码与 OSD 叠加交给 UI 层，
//! 避免 core 依赖图像解码库。

use crate::error::{GcError, Result};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

/// JPEG SOI 标记
const SOI: [u8; 2] = [0xFF, 0xD8];
/// JPEG EOI 标记
const EOI: [u8; 2] = [0xFF, 0xD9];
/// 重组缓冲区上限（8MB，超过则丢弃，防止内存无限增长）
const MAX_PENDING: usize = 8 * 1024 * 1024;

/// 一帧视频帧（原始 JPEG 字节）
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// 帧序号（从 1 起）
    pub seq: u64,
    /// 接收时间戳（ms）
    pub ts_ms: u64,
    /// JPEG 编码数据
    pub jpeg: Vec<u8>,
}

/// 接收统计
#[derive(Debug, Clone, Default)]
pub struct VideoStats {
    /// 成功解析的帧数
    pub frames: u64,
    /// 收到的有效字节数（已扣除垃圾前缀/重组丢弃部分）
    pub bytes: u64,
    /// 因缓冲区溢出被丢弃的帧数
    pub dropped: u64,
}

/// UDP MJPEG 接收器
pub struct VideoReceiver {
    inner: Arc<Mutex<VideoInner>>,
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

struct VideoInner {
    addr: SocketAddr,
    running: bool,
    latest: Option<VideoFrame>,
    stats: VideoStats,
    /// 跨包重组缓冲
    pending: Vec<u8>,
}

impl VideoReceiver {
    /// 绑定 UDP 端口并启动接收任务。
    ///
    /// `addr` 形如 `0.0.0.0:5600`；绑定失败返回 GcError::Io。
    pub async fn start(addr: &str) -> Result<Self> {
        let socket = UdpSocket::bind(addr).await.map_err(GcError::Io)?;
        let addr = socket.local_addr().map_err(GcError::Io)?;
        let inner = Arc::new(Mutex::new(VideoInner {
            addr,
            running: true,
            latest: None,
            stats: VideoStats::default(),
            pending: Vec::new(),
        }));
        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let task_inner = inner.clone();
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 65536];
            loop {
                if *stop_rx.borrow() {
                    break;
                }
                let n = tokio::select! {
                    _ = stop_rx.changed() => break,
                    r = socket.recv(&mut buf) => match r {
                        Ok(n) => n,
                        Err(_) => continue,
                    },
                };
                if n == 0 {
                    continue;
                }
                task_inner.lock().unwrap().feed(&buf[..n]);
            }
            task_inner.lock().unwrap().running = false;
        });
        Ok(Self {
            inner,
            stop: stop_tx,
            task,
        })
    }

    /// 本机绑定地址（测试用它作为发送目标）
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.lock().unwrap().addr
    }

    /// 最新一帧（克隆；无帧时返回 None）
    pub fn latest_frame(&self) -> Option<VideoFrame> {
        self.inner.lock().unwrap().latest.clone()
    }

    /// 接收统计快照
    pub fn stats(&self) -> VideoStats {
        self.inner.lock().unwrap().stats.clone()
    }

    /// 是否仍在接收
    pub fn is_running(&self) -> bool {
        self.inner.lock().unwrap().running
    }

    /// 停止接收（发送停止信号并中止接收任务）
    pub fn stop(&self) {
        let _ = self.stop.send(true);
        self.task.abort();
        self.inner.lock().unwrap().running = false;
    }

    /// 从数据报流中提取完整 JPEG 帧
    ///
    /// 支持：垃圾前缀丢弃、跨包重组、单包多帧。缓冲区超限时清空并丢弃。
    pub fn extract_frames(pending: &mut Vec<u8>, bytes: &[u8], out: &mut Vec<Vec<u8>>) {
        pending.extend_from_slice(bytes);
        if pending.len() > MAX_PENDING {
            pending.clear();
            return;
        }
        loop {
            // 找 SOI；之前的字节视为垃圾丢弃
            let Some(soi) = find_sub(pending, 0, &SOI) else { break };
            if soi > 0 {
                pending.drain(..soi);
            }
            // 在 SOI 之后找 EOI
            let Some(eoi) = find_sub(pending, 2, &EOI) else { break };
            let end = eoi + 2;
            let frame = pending.drain(..end).collect::<Vec<_>>();
            out.push(frame);
        }
    }
}

impl VideoInner {
    fn feed(&mut self, bytes: &[u8]) {
        let mut frames = Vec::new();
        VideoReceiver::extract_frames(&mut self.pending, bytes, &mut frames);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        for f in frames {
            self.stats.bytes += f.len() as u64;
            self.stats.frames += 1;
            self.latest = Some(VideoFrame {
                seq: self.stats.frames,
                ts_ms: now,
                jpeg: f,
            });
        }
    }
}

/// 在 `haystack[from..]` 中查找子序列 `needle`，返回起始下标
fn find_sub(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < from + needle.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 分帧逻辑：垃圾前缀 / 跨包重组 / 单包多帧
    #[test]
    fn extract_basic() {
        let mut pending = Vec::new();
        let mut out = Vec::new();
        let frame = |n: u8| {
            let mut v = vec![0xFF, 0xD8, n, 0x11, 0x22];
            v.extend_from_slice(&[0xFF, 0xD9]);
            v
        };
        let a = frame(1);
        let b = frame(2);

        // 垃圾前缀 + 完整帧
        let mut junk = vec![0x00, 0xAA, 0xBB];
        junk.extend_from_slice(&a);
        VideoReceiver::extract_frames(&mut pending, &junk, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], a);
        assert!(pending.is_empty());

        // 跨包：先发前半（含 SOI），再发后半（含 EOI）
        out.clear();
        let mid = a.len() / 2;
        VideoReceiver::extract_frames(&mut pending, &a[..mid], &mut out);
        assert!(out.is_empty(), "半帧不应产出");
        VideoReceiver::extract_frames(&mut pending, &a[mid..], &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], a);

        // 单包多帧
        out.clear();
        let mut both = b.clone();
        both.extend_from_slice(&a);
        VideoReceiver::extract_frames(&mut pending, &both, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], b);
        assert_eq!(out[1], a);
    }

    /// 缓冲区超限应清空并停止产出
    #[test]
    fn extract_overflow() {
        let mut pending = vec![0xFF; MAX_PENDING]; // 填满缓冲区
        let mut out = Vec::new();
        VideoReceiver::extract_frames(&mut pending, b"more", &mut out);
        assert!(out.is_empty());
        assert!(pending.is_empty());
    }
}
