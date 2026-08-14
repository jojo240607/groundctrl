//! 遥测中枢：链路字节 -> MAVLink 解析 -> 更新 VehicleModel -> 发布总线
//!
//! 每个链路起一个独立任务，互不影响；链路断开自动退出，不影响其他链路。

use std::sync::Arc;

use ::mavlink::common as mav;
use ::mavlink::MavHeader;
use num_traits::FromPrimitive;
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

                        // FENCE_POINT 不在 common dialect，需手写解析（否则会被 parser 丢弃）
                        if let Some(fp) = crate::mlink::fence::decode_fence_point(&bytes) {
                            bus.publish(BusEvent::FencePoint {
                                link: name.clone(),
                                header: ::mavlink::MavHeader {
                                    system_id: bytes.get(5).copied().unwrap_or(0),
                                    component_id: bytes.get(6).copied().unwrap_or(0),
                                    sequence: bytes.get(4).copied().unwrap_or(0),
                                },
                                idx: fp.idx,
                                count: fp.count,
                                lat: fp.lat,
                                lng: fp.lng,
                            });
                        }
                        // MAVLink FTP（FILE_TRANSFER_PROTOCOL）同样不在 common dialect
                        if let Some((_tsys, _tcomp, payload)) = crate::mlink::ftp::decode_ftp(&bytes) {
                            bus.publish(BusEvent::Ftp {
                                link: name.clone(),
                                header: ::mavlink::MavHeader {
                                    system_id: bytes.get(5).copied().unwrap_or(0),
                                    component_id: bytes.get(6).copied().unwrap_or(0),
                                    sequence: bytes.get(4).copied().unwrap_or(0),
                                },
                                payload,
                            });
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

    /// 发送一条 COMMAND_LONG 飞行指令（ARM/DISARM/TAKEOFF/LAND/RTL/DO_SET_MODE 等）
    ///
    /// `params` 为 MAV_CMD 的 7 个参数（param1..param7）。
    pub async fn send_command_long(
        &self,
        sys: u8,
        comp: u8,
        command: mav::MavCmd,
        params: [f32; 7],
    ) -> crate::error::Result<()> {
        if let Some(link) = self.current.lock().await.clone() {
            let msg = mlink::MavMessage::COMMAND_LONG(mav::COMMAND_LONG_DATA {
                param1: params[0],
                param2: params[1],
                param3: params[2],
                param4: params[3],
                param5: params[4],
                param6: params[5],
                param7: params[6],
                command,
                target_system: sys,
                target_component: comp,
                confirmation: 0,
            });
            let header = mlink::default_header();
            self.send_msg(&link, &header, &msg).await?;
        }
        Ok(())
    }

    /// 发送传感器校准指令（MAV_CMD_PREFLIGHT_CALIBRATION）。
    ///
    /// `what` 位掩码：1=陀螺仪 2=加速度计 4=磁罗盘 8=气压计 128=水平校准；0=全部。
    /// 结果通过 COMMAND_ACK 异步回传（见 `VehicleModel::cmd_ack`）。
    pub async fn send_calibrate(
        &self,
        sys: u8,
        comp: u8,
        what: u8,
    ) -> crate::error::Result<()> {
        self.send_command_long(
            sys,
            comp,
            mav::MavCmd::MAV_CMD_PREFLIGHT_CALIBRATION,
            [what as f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        )
        .await
    }

    /// 发送 RC 通道覆盖（RC_CHANNELS_OVERRIDE），用于摇杆 / 键盘操控。
    ///
    /// `chans` 为前 8 个通道值（µs，1000~2000），0 = 不覆盖该通道。
    /// 飞控收到后按 MAVLink 超时机制自动释放（约 3s 无更新即恢复原遥控）。
    pub async fn send_rc_override(
        &self,
        sys: u8,
        comp: u8,
        chans: [u16; 8],
    ) -> crate::error::Result<()> {
        if let Some(link) = self.current.lock().await.clone() {
            let msg = mlink::MavMessage::RC_CHANNELS_OVERRIDE(mav::RC_CHANNELS_OVERRIDE_DATA {
                target_system: sys,
                target_component: comp,
                chan1_raw: chans[0],
                chan2_raw: chans[1],
                chan3_raw: chans[2],
                chan4_raw: chans[3],
                chan5_raw: chans[4],
                chan6_raw: chans[5],
                chan7_raw: chans[6],
                chan8_raw: chans[7],
            });
            let header = mlink::default_header();
            self.send_msg(&link, &header, &msg).await?;
        }
        Ok(())
    }

    /// 清除所有 RC 通道覆盖（全 0 = 不覆盖）
    pub async fn clear_rc_override(&self, sys: u8, comp: u8) -> crate::error::Result<()> {
        self.send_rc_override(sys, comp, [0; 8]).await
    }

    /// 解锁 / 上锁（MAV_CMD_COMPONENT_ARM_DISARM）
    pub async fn send_arm_disarm(&self, sys: u8, comp: u8, armed: bool) -> crate::error::Result<()> {
        self.send_command_long(
            sys,
            comp,
            mav::MavCmd::MAV_CMD_COMPONENT_ARM_DISARM,
            [if armed { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        )
        .await
    }

    /// 一键起飞（MAV_CMD_NAV_TAKEOFF，param7 = 目标高度）
    pub async fn send_takeoff(&self, sys: u8, comp: u8, alt: f32) -> crate::error::Result<()> {
        self.send_command_long(
            sys,
            comp,
            mav::MavCmd::MAV_CMD_NAV_TAKEOFF,
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, alt],
        )
        .await
    }

    /// 降落（MAV_CMD_NAV_LAND）
    pub async fn send_land(&self, sys: u8, comp: u8) -> crate::error::Result<()> {
        self.send_command_long(
            sys,
            comp,
            mav::MavCmd::MAV_CMD_NAV_LAND,
            [0.0; 7],
        )
        .await
    }

    /// 返航（MAV_CMD_NAV_RETURN_TO_LAUNCH）
    pub async fn send_rtl(&self, sys: u8, comp: u8) -> crate::error::Result<()> {
        self.send_command_long(
            sys,
            comp,
            mav::MavCmd::MAV_CMD_NAV_RETURN_TO_LAUNCH,
            [0.0; 7],
        )
        .await
    }

    /// 切换飞行模式（MAV_CMD_DO_SET_MODE，param2 = custom_mode）
    ///
    /// ArduPilot Copter：0=STABILIZE 2=ALT_HOLD 3=AUTO 4=GUIDED 5=LOITER 6=RTL 9=LAND。
    pub async fn send_mode(&self, sys: u8, comp: u8, custom_mode: u32) -> crate::error::Result<()> {
        self.send_command_long(
            sys,
            comp,
            mav::MavCmd::MAV_CMD_DO_SET_MODE,
            [
                1.0, // MAV_MODE_FLAG_CUSTOM_MODE_ENABLED
                custom_mode as f32,
                0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        )
        .await
    }

    /// 发送任意 MAV_CMD（参数以 f32 传入）
    pub async fn send_raw_command(
        &self,
        sys: u8,
        comp: u8,
        cmd_id: u16,
        params: [f32; 7],
    ) -> crate::error::Result<()> {
        let cmd = mav::MavCmd::from_u16(cmd_id);
        match cmd {
            Some(c) => self.send_command_long(sys, comp, c, params).await,
            None => Err(crate::error::GcError::Mavlink(format!(
                "未知 MAV_CMD {cmd_id}"
            ))),
        }
    }

    /// 请求飞控按流率上报数据（REQUEST_DATA_STREAM，ArduPilot 仍支持）
    ///
    /// `rate_hz = 0` 表示停止该流。
    pub async fn request_data_stream(
        &self,
        sys: u8,
        comp: u8,
        stream: mav::MavDataStream,
        rate_hz: u16,
    ) -> crate::error::Result<()> {
        if let Some(link) = self.current.lock().await.clone() {
            let msg = mlink::MavMessage::REQUEST_DATA_STREAM(mav::REQUEST_DATA_STREAM_DATA {
                req_message_rate: rate_hz,
                target_system: sys,
                target_component: comp,
                req_stream_id: stream as u8,
                start_stop: if rate_hz > 0 { 1 } else { 0 },
            });
            let header = mlink::default_header();
            self.send_msg(&link, &header, &msg).await?;
        }
        Ok(())
    }

    /// 按流 ID 请求数据（0=ALL 1=RAW 2=EXT_STATUS 3=RC 4=RAW_CTRL 6=POSITION 10=EXTRA1 11=EXTRA2 12=EXTRA3）
    pub async fn request_data_stream_by_id(
        &self,
        sys: u8,
        comp: u8,
        stream_id: u8,
        rate_hz: u16,
    ) -> crate::error::Result<()> {
        let stream = mav::MavDataStream::from_u8(stream_id);
        match stream {
            Some(s) => self.request_data_stream(sys, comp, s, rate_hz).await,
            None => Err(crate::error::GcError::Mavlink(format!(
                "未知数据流 ID {stream_id}"
            ))),
        }
    }

    /// 按消息间隔设置单条消息的发送频率（MAV_CMD_SET_MESSAGE_INTERVAL，PX4/ArduPilot 通用）
    ///
    /// `interval_us = 0` 停止该消息。
    pub async fn set_message_interval(
        &self,
        sys: u8,
        comp: u8,
        msg_id: u32,
        interval_us: i32,
    ) -> crate::error::Result<()> {
        self.send_command_long(
            sys,
            comp,
            mav::MavCmd::MAV_CMD_SET_MESSAGE_INTERVAL,
            [msg_id as f32, interval_us as f32, 0.0, 0.0, 0.0, 0.0, 0.0],
        )
        .await
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

    /// 上传地理围栏：逐条发送 FENCE_POINT（坐标单位度）。
    ///
    /// 围栏需至少 3 个点（多边形闭合由飞控处理）。间隔 10ms 防真实链路丢帧。
    pub async fn upload_fence(
        &self,
        sys: u8,
        comp: u8,
        points: &[(f64, f64)],
    ) -> crate::error::Result<()> {
        if points.len() < 3 {
            return Err(crate::error::GcError::Mavlink(
                "围栏至少需要 3 个点".into(),
            ));
        }
        let count = points.len().min(u8::MAX as usize) as u8;
        if let Some(link) = self.current_link().await {
            let header = mlink::default_header();
            for (idx, (lat, lng)) in points.iter().take(count as usize).enumerate() {
                let bytes = crate::mlink::fence::encode_fence_point(
                    &header,
                    sys,
                    comp,
                    idx as u8,
                    count,
                    (lat * 1e7) as i32,
                    (lng * 1e7) as i32,
                )?;
                self.send_raw(&link, &bytes).await?;
                // 防止真实串口/数传链路丢帧
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
        Ok(())
    }

    /// 下载地理围栏：先请求第 0 点得知总数，再逐条请求（FENCE_FETCH_POINT）。
    ///
    /// 返回 (纬度, 经度) 列表（单位度）。任一步 3s 超时即返回已收到的部分。
    pub async fn download_fence(
        &self,
        sys: u8,
        comp: u8,
    ) -> crate::error::Result<Vec<(f64, f64)>> {
        let link = match self.current_link().await {
            Some(l) => l,
            None => return Ok(Vec::new()),
        };
        let header = mlink::default_header();
        let mut rx = self.bus.subscribe();

        // 1) 请求第 0 点，获取 count
        let fetch0 = crate::mlink::fence::encode_fence_fetch_point(&header, sys, comp, 0)?;
        self.send_raw(&link, &fetch0).await?;
        let (count, first) = match self.recv_fence_point(&mut rx, sys, 0).await {
            Some(v) => v,
            None => return Ok(Vec::new()), // 超时：无围栏或飞控无响应
        };

        // 2) 逐条请求（idx=0 已收到，从 1 开始）并收集
        let mut pts: Vec<(f64, f64)> = Vec::with_capacity(count as usize);
        pts.push((first.0 as f64 / 1e7, first.1 as f64 / 1e7));
        for idx in 1..count {
            let req = crate::mlink::fence::encode_fence_fetch_point(&header, sys, comp, idx as u8)?;
            self.send_raw(&link, &req).await?;
            match self.recv_fence_point(&mut rx, sys, idx as u8).await {
                Some((_count, (lat_e7, lng_e7))) => {
                    pts.push((lat_e7 as f64 / 1e7, lng_e7 as f64 / 1e7));
                }
                None => break, // 该条超时，停止后续请求
            }
        }
        Ok(pts)
    }

    /// 等待指定 idx 的 FENCE_POINT（带 3s 超时），返回 (count, (lat_e7, lng_e7))
    async fn recv_fence_point(
        &self,
        rx: &mut tokio::sync::broadcast::Receiver<crate::proto::bus::BusEvent>,
        sys: u8,
        idx: u8,
    ) -> Option<(u8, (i32, i32))> {
        let deadline = tokio::time::Duration::from_secs(3);
        match tokio::time::timeout(deadline, async {
            loop {
                match rx.recv().await {
                    Ok(crate::proto::bus::BusEvent::FencePoint {
                        header, idx: i, count, lat, lng, ..
                    }) => {
                        if header.system_id == sys && i == idx {
                            return Some((count, (lat, lng)));
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

    /// 发送一条 MAVLink FTP 帧（FILE_TRANSFER_PROTOCOL，手写编解码）
    pub async fn send_ftp(
        &self,
        sys: u8,
        comp: u8,
        p: &crate::mlink::ftp::FtpPayload,
    ) -> crate::error::Result<()> {
        if let Some(link) = self.current.lock().await.clone() {
            let header = mlink::default_header();
            let bytes = crate::mlink::ftp::encode_ftp(&header, sys, comp, p)?;
            self.send_raw(&link, &bytes).await?;
        }
        Ok(())
    }

    /// 发送 FTP 请求并等待匹配应答（ACK/NACK），3s 超时
    ///
    /// 应答匹配条件：seq 相同且 req_opcode == 请求的 opcode。
    async fn ftp_exchange(
        &self,
        sys: u8,
        comp: u8,
        req: &crate::mlink::ftp::FtpPayload,
    ) -> crate::error::Result<crate::mlink::ftp::FtpPayload> {
        let mut rx = self.bus.subscribe();
        self.send_ftp(sys, comp, req).await?;
        let expect = req.opcode;
        let resp = match tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            async {
                loop {
                    match rx.recv().await {
                        Ok(BusEvent::Ftp { payload, .. }) => {
                            if payload.seq == req.seq && payload.req_opcode == expect {
                                return Ok(payload);
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => {
                            return Err(crate::error::GcError::Link("ftp bus closed".into()))
                        }
                    }
                }
            },
        )
        .await
        {
            Ok(r) => r?,
            Err(_) => {
                return Err(crate::error::GcError::Mavlink(format!(
                    "FTP 请求 opcode={expect} 超时"
                )))
            }
        };
        if resp.is_nack() {
            return Err(crate::error::GcError::Mavlink(format!(
                "FTP 请求 opcode={expect} 被拒绝, err={}",
                resp.err()
            )));
        }
        Ok(resp)
    }

    /// 通过 MAVLink FTP 上传文件到飞控（固件升级）：CreateFile + 逐块 WriteFile + TerminateSession。
    ///
    /// `on_progress(sent, total)` 每写入一块回调一次（总字节数）。
    pub async fn upload_firmware<F>(
        &self,
        sys: u8,
        comp: u8,
        name: &str,
        data: &[u8],
        mut on_progress: F,
    ) -> crate::error::Result<()>
    where
        F: FnMut(u64, u64),
    {
        use crate::mlink::ftp::*;
        if data.is_empty() {
            return Err(crate::error::GcError::Mavlink("固件内容为空".into()));
        }
        if name.is_empty() || name.len() > DATA_MAX {
            return Err(crate::error::GcError::Mavlink("文件名非法".into()));
        }
        let total = data.len() as u64;
        let mut seq = 0u16;
        on_progress(0, total);

        // 1) 创建文件，拿写会话
        let create = FtpPayload::request(seq, 0, OP_CREATE_FILE, name.as_bytes().to_vec());
        let ack = self.ftp_exchange(sys, comp, &create).await?;
        let session = ack.session;
        seq = seq.wrapping_add(1);

        // 2) 逐块写入（每块最多 226 字节，data 前 8 字节为 offset）
        let mut sent: u64 = 0;
        while sent < total {
            let start = sent as usize;
            let end = (start + WRITE_CHUNK_MAX).min(data.len());
            let mut chunk = Vec::with_capacity(WRITE_CHUNK_MAX + 8);
            chunk.extend_from_slice(&sent.to_le_bytes());
            chunk.extend_from_slice(&data[start..end]);
            let wr = FtpPayload::request(seq, session, OP_WRITE_FILE, chunk);
            self.ftp_exchange(sys, comp, &wr).await?;
            seq = seq.wrapping_add(1);
            sent = end as u64;
            on_progress(sent, total);
        }

        // 3) 结束写会话（不等待应答，避免最后一块 ACK 丢失导致误报）
        let term = FtpPayload::request(seq, session, OP_TERMINATE_SESSION, Vec::new());
        let _ = self.send_ftp(sys, comp, &term).await;
        Ok(())
    }

    /// 通过 MAVLink FTP 从飞控下载文件：OpenFileRO + 循环 ReadFile + TerminateSession。
    ///
    /// 读取至 EOF（短块或空块）结束。
    pub async fn download_firmware(
        &self,
        sys: u8,
        comp: u8,
        name: &str,
    ) -> crate::error::Result<Vec<u8>> {
        use crate::mlink::ftp::*;
        let mut seq = 0u16;

        // 1) 打开文件，拿读会话
        let open = FtpPayload::request(seq, 0, OP_OPEN_FILE_RO, name.as_bytes().to_vec());
        let ack = self.ftp_exchange(sys, comp, &open).await?;
        let session = ack.session;
        seq = seq.wrapping_add(1);

        // 2) 循环读取（每次最多 234 字节）
        let mut out = Vec::new();
        let mut offset = 0u64;
        loop {
            let mut rd = FtpPayload::request(seq, session, OP_READ_FILE, Vec::new());
            rd.offset = offset;
            let ack = self.ftp_exchange(sys, comp, &rd).await?;
            seq = seq.wrapping_add(1);
            if ack.data.is_empty() || ack.data.len() < DATA_MAX {
                out.extend_from_slice(&ack.data);
                break; // 末尾短块 / 空块
            }
            offset += ack.data.len() as u64;
            out.extend_from_slice(&ack.data);
        }

        let term = FtpPayload::request(seq, session, OP_TERMINATE_SESSION, Vec::new());
        let _ = self.send_ftp(sys, comp, &term).await;
        Ok(out)
    }

    /// 列出飞控 FTP 根目录，返回 (条目类型, 文件名) 列表。
    ///
    /// 条目类型见 `mlink::ftp::FILETYPE_*`。
    pub async fn ftp_list_directory(
        &self,
        sys: u8,
        comp: u8,
    ) -> crate::error::Result<Vec<(u8, String)>> {
        use crate::mlink::ftp::*;
        let mut seq = 0u16;
        let mut entries = Vec::new();
        let mut offset = 0u64;
        loop {
            let mut lst = FtpPayload::request(seq, 0, OP_LIST_DIRECTORY, Vec::new());
            lst.offset = offset;
            let ack = self.ftp_exchange(sys, comp, &lst).await?;
            seq = seq.wrapping_add(1);
            if ack.data.is_empty() {
                break;
            }
            // data 布局：每项 [类型 u8][文件名以 NUL 结尾]
            let mut i = 0;
            while i < ack.data.len() {
                let t = ack.data[i];
                i += 1;
                let name_end = ack.data[i..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|p| i + p)
                    .unwrap_or(ack.data.len());
                let nm = String::from_utf8_lossy(&ack.data[i..name_end]).to_string();
                entries.push((t, nm));
                i = name_end + 1;
            }
            if ack.data.len() < DATA_MAX {
                break;
            }
            offset += ack.data.len() as u64;
        }
        Ok(entries)
    }

    /// 计算飞控侧文件的 CRC32（OP_CALC_FILE_CRC32），用于固件完整性校验。
    pub async fn ftp_file_crc32(&self, sys: u8, comp: u8, name: &str) -> crate::error::Result<u32> {
        use crate::mlink::ftp::*;
        let req = FtpPayload::request(0, 0, OP_CALC_FILE_CRC32, name.as_bytes().to_vec());
        let ack = self.ftp_exchange(sys, comp, &req).await?;
        if ack.data.len() < 4 {
            return Err(crate::error::GcError::Mavlink("CRC32 应答数据不足".into()));
        }
        Ok(u32::from_le_bytes(ack.data[..4].try_into().unwrap()))
    }

    /// 删除飞控侧文件（OP_REMOVE_FILE）
    pub async fn ftp_remove_file(
        &self,
        sys: u8,
        comp: u8,
        name: &str,
    ) -> crate::error::Result<()> {
        use crate::mlink::ftp::*;
        let req = FtpPayload::request(0, 0, OP_REMOVE_FILE, name.as_bytes().to_vec());
        self.ftp_exchange(sys, comp, &req).await?;
        Ok(())
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
