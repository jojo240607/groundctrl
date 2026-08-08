//! 遥测中枢：链路字节 -> MAVLink 解析 -> 更新 VehicleModel -> 发布总线
//!
//! 每个链路起一个独立任务，互不影响；链路断开自动退出，不影响其他链路。

use std::sync::Arc;

use ::mavlink::common as mav;
use ::mavlink::MavHeader;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::link::LinkHandle;
use crate::mlink;
use crate::mlink::MavlinkParser;
use crate::proto::bus::{Bus, BusEvent};
use crate::services::alarms::{Alarm, FlightMonitor};
use crate::services::log::LogManager;
use crate::vehicle::mission::MissionPlanner;
use crate::vehicle::params::ParamManager;
use crate::vehicle::VehicleModel;

/// 遥测中枢句柄
///
/// 手动实现 `Clone`：`tokio::sync::Mutex` 不可 `Clone`，且 `active`/`current`
/// 属于“运行态”句柄（活动任务 / 当前链路），克隆出的副本只用于订阅与查询，
/// 不应抢占原句柄持有的链路，故克隆时重置为空 `Mutex`。
pub struct TelemetryHub {
    bus: Bus,
    vehicles: Arc<Mutex<std::collections::HashMap<u8, VehicleModel>>>,
    telem_tx: tokio::sync::broadcast::Sender<VehicleModel>,
    /// 当前活动链路的消费任务（用于断开时取消）
    active: Mutex<Option<JoinHandle<()>>>,
    /// 当前活动链路句柄（用于显示名称 / 重连判断）
    current: Mutex<Option<LinkHandle>>,
    /// 每架飞机的参数缓存（按 system_id）
    params: Arc<Mutex<std::collections::HashMap<u8, ParamManager>>>,
    /// 飞行告警监控器
    monitor: Arc<Mutex<FlightMonitor>>,
    /// 飞行日志
    log: Arc<Mutex<LogManager>>,
    /// 地面站侧的航点缓存（上传时写入，下载时返回）
    mission: Mutex<MissionPlanner>,
}

impl Clone for TelemetryHub {
    fn clone(&self) -> Self {
        Self {
            bus: self.bus.clone(),
            vehicles: self.vehicles.clone(),
            telem_tx: self.telem_tx.clone(),
            active: Mutex::new(None),
            current: Mutex::new(None),
            params: self.params.clone(),
            monitor: self.monitor.clone(),
            log: self.log.clone(),
            mission: Mutex::new(MissionPlanner::new()),
        }
    }
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
            params: Arc::new(Mutex::new(std::collections::HashMap::new())),
            monitor: Arc::new(Mutex::new(FlightMonitor::with_defaults())),
            log: Arc::new(Mutex::new(LogManager::new())),
            mission: Mutex::new(MissionPlanner::new()),
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

    /// 取某架飞机的参数管理器（不存在则新建）
    pub async fn param_manager(&self, sys: u8) -> ParamManager {
        self.params.lock().await.entry(sys).or_default().clone()
    }

    /// 飞行日志管理器
    pub fn log(&self) -> Arc<Mutex<LogManager>> {
        self.log.clone()
    }

    /// 接入一条链路，启动解析任务。返回任务句柄。
    pub fn attach(&self, link: LinkHandle) -> tokio::task::JoinHandle<()> {
        let bus = self.bus.clone();
        let vehicles = self.vehicles.clone();
        let telem_tx = self.telem_tx.clone();
        let name = link.name();
        let params = self.params.clone();
        let monitor = self.monitor.clone();
        let log = self.log.clone();

        tokio::spawn(async move {
            let mut parser = MavlinkParser::new();
            // 默认开启实时日志记录（可被 set_logging 关闭）
            log.lock().await.set_recording(true);
            tracing::info!("link {name} attached");
            bus.publish(BusEvent::LinkState {
                link: name.clone(),
                open: true,
            });

            loop {
                match link.recv().await {
                    Ok(bytes) => {
                        // 记录原始帧到日志
                        let ts_ms = crate::vehicle::now_ms();
                        log.lock().await.record(ts_ms, &bytes);

                        match parser.feed(&bytes) {
                            Ok(frames) => {
                                for (header, msg) in frames {
                                    bus.publish(BusEvent::Mavlink {
                                        link: name.clone(),
                                        header,
                                        msg: msg.clone(),
                                    });
                                    // 更新参数缓存
                                    if let ::mavlink::common::MavMessage::PARAM_VALUE(d) = &msg {
                                        let sys = header.system_id;
                                        let just_done = {
                                            let mut pm = params.lock().await;
                                            let mgr = pm.entry(sys).or_default();
                                            mgr.apply_param_value(d)
                                        };
                                        let pm = params.lock().await;
                                        if let Some(mgr) = pm.get(&sys) {
                                            bus.publish(BusEvent::Params {
                                                link: name.clone(),
                                                complete: mgr.is_complete(),
                                                received: mgr.len() as u16,
                                                expected: mgr.expected(),
                                                entries: mgr.list(),
                                            });
                                        }
                                        drop(pm);
                                        let _ = just_done;
                                    }
                                    // 更新对应飞机模型
                                    let sys = header.system_id;
                                    let mut map = vehicles.lock().await;
                                    let vm = map.entry(sys).or_default();
                                    vm.link_name = name.clone();
                                    vm.apply(&header, &msg);
                                    let snapshot = vm.clone();
                                    // 告警评估
                                    let alarms: Vec<Alarm> = {
                                        let mut mon = monitor.lock().await;
                                        mon.evaluate(&snapshot)
                                    };
                                    drop(map);
                                    let _ = telem_tx.send(snapshot);
                                    for a in alarms {
                                        bus.publish(BusEvent::Alarm {
                                            link: name.clone(),
                                            alarm: a,
                                        });
                                    }
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

    /// 当前活动链路句柄（无连接时返回 None）
    pub async fn current_link(&self) -> Option<LinkHandle> {
        self.current.lock().await.clone()
    }

    /// 发送原始 MAVLink 帧到指定链路
    pub async fn send_raw(&self, link: &LinkHandle, bytes: &[u8]) -> crate::error::Result<()> {
        link.send(bytes).await
    }

    /// 发送一条消息（编码后下发）
    pub async fn send_msg(
        &self,
        link: &LinkHandle,
        header: &MavHeader,
        msg: &mlink::MavMessage,
    ) -> crate::error::Result<()> {
        let bytes = mlink::encode_v2(header, msg)?;
        self.send_raw(link, &bytes).await
    }

    /// 请求飞控上报全部参数（向当前链路发送 PARAM_REQUEST_LIST）
    pub async fn request_params(&self, sys: u8, comp: u8) -> crate::error::Result<()> {
        if let Some(link) = self.current.lock().await.clone() {
            let msg = ParamManager::make_request_list(sys, comp);
            let header = mlink::default_header();
            self.send_msg(&link, &header, &msg).await?;
            // 清空旧缓存，准备重新拉取
            self.params.lock().await.entry(sys).or_default().clear();
        }
        Ok(())
    }

    /// 写回单个参数（向当前链路发送 PARAM_SET）
    pub async fn set_param(
        &self,
        sys: u8,
        comp: u8,
        name: &str,
        value: f32,
    ) -> crate::error::Result<()> {
        if let Some(link) = self.current.lock().await.clone() {
            let msg = ParamManager::make_set(sys, comp, name, value);
            let header = mlink::default_header();
            self.send_msg(&link, &header, &msg).await?;
            // 乐观更新缓存
            self.params.lock().await.entry(sys).or_default().set_local(name, value);
        }
        Ok(())
    }

    /// 开始 / 停止飞行日志记录
    pub async fn set_logging(&self, on: bool) {
        self.log.lock().await.set_recording(on);
    }

    /// 当前日志帧数
    pub async fn log_frames(&self) -> usize {
        self.log.lock().await.len()
    }

    /// 上传航点：先发 MISSION_COUNT，再逐条发 MISSION_ITEM_INT
    pub async fn upload_mission(
        &self,
        sys: u8,
        comp: u8,
        items: &[crate::vehicle::mission::Waypoint],
    ) -> crate::error::Result<()> {
        if let Some(link) = self.current_link().await {
            let header = mlink::default_header();
            // MISSION_COUNT
            let planner = crate::vehicle::mission::MissionPlanner::from_items(items);
            let count_msg = planner.make_count(sys, comp);
            self.send_msg(&link, &header, &count_msg).await?;
            // 逐条 ITEM
            for wp in items {
                let item_msg = wp.to_item_int(sys, comp);
                self.send_msg(&link, &header, &item_msg).await?;
            }
        }
        // 缓存到地面站侧，供 download_mission 返回
        *self.mission.lock().await = crate::vehicle::mission::MissionPlanner::from_items(items);
        Ok(())
    }

    /// 下载航点：向飞控发 MISSION_REQUEST_LIST，逐条收 MISSION_ITEM_INT，返回完整航点。
    ///
    /// 协议握手：REQUEST_LIST -> (COUNT) -> 对每条 REQUEST(seq) -> (ITEM_INT(seq))，
    /// 全部收齐后写入地面站侧缓存并返回。任何一步超时则返回已收到的部分（至少 0 条）。
    pub async fn download_mission(
        &self,
        sys: u8,
        comp: u8,
    ) -> crate::error::Result<Vec<crate::vehicle::mission::Waypoint>> {
        let link = match self.current_link().await {
            Some(l) => l,
            None => return Ok(Vec::new()),
        };
        let header = mlink::default_header();

        // 订阅总线，独立接收飞控回传（不干扰 attach 任务的常规处理）
        let mut rx = self.bus.subscribe();

        // 1) 请求航点总数
        let req_list = crate::vehicle::mission::MissionPlanner::make_request_list(sys, comp);
        self.send_msg(&link, &header, &req_list).await?;

        let count = match self.recv_mission_count(&mut rx, sys).await {
            Some(c) => c,
            None => return Ok(Vec::new()), // 超时：无航点或飞控无响应
        };

        // 2) 逐条请求并接收
        let mut items: Vec<mav::MISSION_ITEM_INT_DATA> = Vec::with_capacity(count as usize);
        for seq in 0..count {
            let req = crate::vehicle::mission::MissionPlanner::make_request(sys, comp, seq);
            self.send_msg(&link, &header, &req).await?;
            if let Some(data) = self.recv_mission_item(&mut rx, sys, seq).await {
                items.push(data);
            } else {
                break; // 该条超时，停止后续请求
            }
        }

        let planner = crate::vehicle::mission::MissionPlanner::from_int_items(&items);
        let wps = planner.items().to_vec();
        *self.mission.lock().await = planner;
        Ok(wps)
    }

    /// 等待 MISSION_COUNT（带 3s 超时），返回航点总数
    async fn recv_mission_count(
        &self,
        rx: &mut tokio::sync::broadcast::Receiver<crate::proto::bus::BusEvent>,
        sys: u8,
    ) -> Option<u16> {
        let deadline = tokio::time::Duration::from_secs(3);
        match tokio::time::timeout(deadline, async {
            loop {
                match rx.recv().await {
                    Ok(crate::proto::bus::BusEvent::Mavlink { header, msg, .. }) => {
                        if header.system_id != sys {
                            continue;
                        }
                        if let mav::MavMessage::MISSION_COUNT(d) = &msg {
                            if d.target_system == sys {
                                return Some(d.count);
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .await
        {
            Ok(v) => v,
            Err(_) => None,
        }
    }

    /// 等待指定 seq 的 MISSION_ITEM_INT（带 3s 超时）
    async fn recv_mission_item(
        &self,
        rx: &mut tokio::sync::broadcast::Receiver<crate::proto::bus::BusEvent>,
        sys: u8,
        seq: u16,
    ) -> Option<mav::MISSION_ITEM_INT_DATA> {
        let deadline = tokio::time::Duration::from_secs(3);
        match tokio::time::timeout(deadline, async {
            loop {
                match rx.recv().await {
                    Ok(crate::proto::bus::BusEvent::Mavlink { header, msg, .. }) => {
                        if header.system_id != sys {
                            continue;
                        }
                        if let mav::MavMessage::MISSION_ITEM_INT(d) = &msg {
                            if d.target_system == sys && d.seq == seq {
                                return Some(d.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .await
        {
            Ok(v) => v,
            Err(_) => None,
        }
    }

    /// 返回地面站侧缓存的航点（即最近一次编辑/上传、或下载的航点）。
    pub async fn get_mission(&self) -> Vec<crate::vehicle::mission::Waypoint> {
        self.mission.lock().await.items().to_vec()
    }
    /// 运行时更新告警监控阈值（用户在设置面板编辑告警规则后调用）
    pub async fn set_monitor_config(&self, cfg: crate::services::alarms::MonitorConfig) {
        self.monitor.lock().await.set_config(cfg);
    }

    /// 读取当前告警规则配置。
    pub async fn monitor_config(&self) -> crate::services::alarms::MonitorConfig {
        self.monitor.lock().await.config().clone()
    }
}

impl Default for TelemetryHub {
    fn default() -> Self {
        Self::new()
    }
}
