//! 遥测中枢：链路字节 -> MAVLink 解析 -> 更新 VehicleModel -> 发布总线
//!
//! 每个链路起一个独立任务，互不影响；链路断开自动退出，不影响其他链路。

use std::sync::Arc;

use tokio::sync::Mutex;

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
}

impl TelemetryHub {
    pub fn new() -> Self {
        let bus = Bus::new(1024);
        let (telem_tx, _rx) = tokio::sync::broadcast::channel(64);
        Self {
            bus,
            vehicles: Arc::new(Mutex::new(std::collections::HashMap::new())),
            telem_tx,
        }
    }

    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    pub fn subscribe_telemetry(&self) -> tokio::sync::broadcast::Receiver<VehicleModel> {
        self.telem_tx.subscribe()
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
