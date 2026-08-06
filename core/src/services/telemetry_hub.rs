//! 遥测中枢：链路字节 -> MAVLink 解析 -> 更新 VehicleModel -> 发布总线
//!
//! 每个链路起一个独立任务，互不影响；链路断开自动退出，不影响其他链路。

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::link::LinkHandle;
use crate::mlink;
use crate::mlink::MavlinkParser;
use crate::proto::bus::{Bus, BusEvent};
use crate::vehicle::VehicleModel;

/// 遥测中枢句柄
pub struct TelemetryHub {
    bus: Bus,
    vehicles: Arc<Mutex<std::collections::HashMap<u8, VehicleModel>>>,
    telem_tx: tokio::sync::broadcast::Sender<VehicleModel>,
    /// 当前活动链路的消费任务（用于断开时取消）
    active: Mutex<Option<JoinHandle<()>>>,
    /// 当前活动链路句柄（用于显示名称 / 重连判断）
    current: Mutex<Option<LinkHandle>>,
}

impl TelemetryHub {
    pub fn new() -> Self {
        let bus = Bus::new(1024);
        let (telem_tx, _rx) = tokio::sync::broadcast::channel(64);
        Self {
            bus,
            vehicles: Arc::new(Mutex::new(std::collections::HashMap::new())),
            telem_tx,
            active: Mutex::new(None),
            current: Mutex::new(None),
        }
    }

    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    pub fn subscribe_telemetry(&self) -> tokio::sync::broadcast::Receiver<VehicleModel> {
        self.telem_tx.subscribe()
    }

    /// 当前机队中所有飞机的快照（按 system_id 聚合）
    pub async fn fleet(&self) -> Vec<VehicleModel> {
        self.vehicles.lock().await.values().cloned().collect()
    }

    /// 接入一条链路，启动解析任务。返回任务句柄。
    pub fn attach(&self, link: LinkHandle) -> tokio::task::JoinHandle<()> {
        let bus = self.bus.clone();
        let vehicles = self.vehicles.clone();
        let telem_tx = self.telem_tx.clone();
        let name = link.name();

        tokio::spawn(async move {
            let mut parser = MavlinkParser::new();
            tracing::info!("link {name} attached");
            bus.publish(BusEvent::LinkState {
                link: name.clone(),
                open: true,
            });

            loop {
                match link.recv().await {
                    Ok(bytes) => {
                        match parser.feed(&bytes) {
                            Ok(frames) => {
                                for (header, msg) in frames {
                                    bus.publish(BusEvent::Mavlink {
                                        link: name.clone(),
                                        header,
                                        msg: msg.clone(),
                                    });
                                    // 更新对应飞机模型
                                    let sys = header.system_id;
                                    let mut map = vehicles.lock().await;
                                    let vm = map.entry(sys).or_default();
                                    vm.link_name = name.clone();
                                    vm.apply(&header, &msg);
                                    let snapshot = vm.clone();
                                    drop(map);
                                    let _ = telem_tx.send(snapshot);
                                }
                            }
                            Err(e) => tracing::warn!("parse error: {e}"),
                        }
                    }
                    Err(e) => {
                        tracing::warn!("link {name} recv error: {e}");
                        break;
                    }
                }
            }

            bus.publish(BusEvent::LinkState {
                link: name.clone(),
                open: false,
            });
            tracing::info!("link {name} detached");
        })
    }

    /// 接入一条链路并设为「当前活动链路」，会先断开旧链路。
    ///
    /// 多次连接时只有一条活动链路；切换链路会自动取消上一条的解析任务。
    pub async fn connect(&self, link: LinkHandle) {
        self.disconnect().await;

        let name = link.name();
        let handle = self.attach(link.clone());
        *self.active.lock().await = Some(handle);
        *self.current.lock().await = Some(link);
        self.bus.publish(BusEvent::LinkState {
            link: name,
            open: true,
        });
        tracing::info!("telemetry hub active link set");
    }

    /// 断开当前活动链路：取消解析任务并清空当前句柄。
    pub async fn disconnect(&self) {
        // 取消旧任务
        if let Some(h) = self.active.lock().await.take() {
            h.abort();
        }
        // 通知链路状态
        if let Some(old) = self.current.lock().await.take() {
            let name = old.name();
            self.bus.publish(BusEvent::LinkState {
                link: name,
                open: false,
            });
        }
        tracing::info!("telemetry hub disconnected");
    }

    /// 当前活动链路名称（无连接时返回 None）
    pub async fn current_link_name(&self) -> Option<String> {
        self.current.lock().await.as_ref().map(|l| l.name())
    }

    /// 发送原始 MAVLink 帧到指定链路
    pub async fn send_raw(&self, link: &LinkHandle, bytes: &[u8]) -> crate::error::Result<()> {
        link.send(bytes).await
    }

    /// 发送一条消息（编码后下发）
    pub async fn send_msg(
        &self,
        link: &LinkHandle,
        header: &::mavlink::MavHeader,
        msg: &mlink::MavMessage,
    ) -> crate::error::Result<()> {
        let bytes = mlink::encode_v2(header, msg)?;
        self.send_raw(link, &bytes).await
    }
}

impl Default for TelemetryHub {
    fn default() -> Self {
        Self::new()
    }
}
