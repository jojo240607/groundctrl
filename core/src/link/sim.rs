//! 模拟链路（SITL / 回环测试）
//!
//! 向订阅者周期性推送合成心跳 + 姿态 + 电量 + GPS 消息，用于无硬件时验证全链路。
//! 同时响应 GCS 下行：
//! - 收到 `PARAM_REQUEST_LIST` 时，回放一组示例 `PARAM_VALUE`
//! - 电量随时间缓慢下降，用于验证告警逻辑

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};

use mavlink_core::common as mav;
use crate::error::{GcError, Result};
use crate::link::{Link, LinkQuality, LinkStats};
use crate::mlink;
use crate::vehicle::params::string_to_cstr;

/// 模拟飞控 FTP 会话
#[derive(Clone)]
struct FtpSimSession {
    path: String,
    offset: u64,
}

/// 模拟链路内部可变状态
struct SimState {
    /// 电量百分比（随时间下降）
    battery_pct: i8,
    /// 已存储的围栏点（FENCE_POINT 坐标，单位 1e7 度）
    fence: Vec<(i32, i32)>,
    /// GCS 下发的 RC 通道覆盖（0 = 不覆盖，1000~2000 = 覆盖值）
    rc_override: [u16; 8],
    /// 模拟闪存：文件名 -> 内容（MAVLink FTP 固件升级用）
    ftp_files: std::collections::HashMap<String, Vec<u8>>,
    /// 打开的 FTP 会话（id -> 会话）
    ftp_sessions: Vec<(u8, FtpSimSession)>,
    /// 下一个会话 id
    ftp_next_session: u8,
}

impl SimState {
    fn new() -> Self {
        Self {
            battery_pct: 80,
            fence: Vec::new(),
            rc_override: [0; 8],
            ftp_files: std::collections::HashMap::new(),
            ftp_sessions: Vec::new(),
            ftp_next_session: 1,
        }
    }
}

pub struct SimLink {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    stats: LinkStats,
    header: mav::MavHeader,
    /// 模拟飞控侧围栏存储（与周期任务共享）
    state: Arc<tokio::sync::Mutex<SimState>>,
}

impl SimLink {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let producer = tx.clone();
        let state = Arc::new(Mutex::new(SimState::new()));
        let st = state.clone();

        // 模拟飞控的 system/component id
        let header = mav::MavHeader {
            system_id: 1,
            component_id: 1,
            sequence: 0,
        };

        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        tokio::spawn(async move {
            let enc = |msg: mlink::MavMessage| -> Vec<u8> {
                match mlink::encode_v2(&header, &msg) {
                    Ok(b) => b,
                    Err(_) => Vec::new(),
                }
            };

            let mut tick = tokio::time::interval(Duration::from_millis(100));
            loop {
                tick.tick().await;
                let t = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs_f64();
                // 相对启动时刻的运行秒数（用于平滑变化）
                let elapsed_sec = t - start;

                // 电量每 ~2s 降 1%（用取模把范围限制在 [5,80]，避免溢出）
                {
                    let mut s = st.lock().await;
                    let drop = ((elapsed_sec as i32) / 2) % 80; // 每 2s 计 1, 最多降 80
                    let new_pct = 80 - drop;
                    s.battery_pct = if new_pct < 5 { 5 } else { new_pct as i8 };
                }

                let hb = mlink::MavMessage::HEARTBEAT(mav::HEARTBEAT_DATA {
                    custom_mode: 0,
                    mavtype: mav::MavType::MAV_TYPE_QUADROTOR,
                    autopilot: mav::MavAutopilot::MAV_AUTOPILOT_ARDUPILOTMEGA,
                    base_mode: mav::MavModeFlag::MAV_MODE_FLAG_CUSTOM_MODE_ENABLED,
                    system_status: mav::MavState::MAV_STATE_ACTIVE,
                    mavlink_version: 3,
                });
                let _ = producer.send(enc(hb));

                let att = mlink::MavMessage::ATTITUDE(mav::ATTITUDE_DATA {
                    time_boot_ms: (t * 1000.0) as u32,
                    roll: (t * 0.5).sin() as f32 * 0.3,
                    pitch: (t * 0.3).cos() as f32 * 0.2,
                    yaw: (t * 0.1) as f32,
                    rollspeed: 0.0,
                    pitchspeed: 0.0,
                    yawspeed: 0.0,
                });
                let _ = producer.send(enc(att));

                let battery_pct = st.lock().await.battery_pct;
                let bat = mlink::MavMessage::SYS_STATUS(mav::SYS_STATUS_DATA {
                    onboard_control_sensors_present: mav::MavSysStatusSensor::empty(),
                    onboard_control_sensors_enabled: mav::MavSysStatusSensor::empty(),
                    onboard_control_sensors_health: mav::MavSysStatusSensor::empty(),
                    load: 100,
                    voltage_battery: 12000,
                    current_battery: -1,
                    battery_remaining: battery_pct,
                    drop_rate_comm: 0,
                    errors_comm: 0,
                    errors_count1: 0,
                    errors_count2: 0,
                    errors_count3: 0,
                    errors_count4: 0,
                });
                let _ = producer.send(enc(bat));

                let gps = mlink::MavMessage::GLOBAL_POSITION_INT(mav::GLOBAL_POSITION_INT_DATA {
                    time_boot_ms: (t * 1000.0) as u32,
                    lat: 311_000_000 + ((t * 10.0) as i32 % 100_000),
                    lon: 121_400_000 + ((t * 10.0) as i32 % 100_000),
                    alt: 10000,
                    relative_alt: 1000,
                    vx: 0,
                    vy: 0,
                    vz: 0,
                    hdg: (t * 10.0) as u16 % 360,
                });
                let _ = producer.send(enc(gps));

                // GPS 原始数据（精度 / 速度，用于 GPS 详情卡片）
                let gps_raw = mlink::MavMessage::GPS_RAW_INT(mav::GPS_RAW_INT_DATA {
                    time_usec: (t * 1_000_000.0) as u64,
                    lat: 311_000_000 + ((t * 10.0) as i32 % 100_000),
                    lon: 121_400_000 + ((t * 10.0) as i32 % 100_000),
                    alt: 10000,
                    eph: 80 + ((t * 3.0) as u16 % 40), // 0.8~1.2m
                    epv: 150,
                    vel: 1500, // 15 m/s
                    cog: (t * 10.0) as u16 % 36000,
                    fix_type: mav::GpsFixType::GPS_FIX_TYPE_3D_FIX,
                    satellites_visible: 12 + ((t * 7.0) as u8 % 4),
                    alt_ellipsoid: 0,
                    h_acc: 1000,
                    v_acc: 2000,
                    vel_acc: 500,
                    hdg_acc: 100,
                    yaw: 0,
                });
                let _ = producer.send(enc(gps_raw));

                // 空速 / 地速 / 垂直速度 / 油门（合成，用于 HUD 仪表验证）
                let vfr = mlink::MavMessage::VFR_HUD(mav::VFR_HUD_DATA {
                    airspeed: 15.0 + (t * 0.7).sin() as f32 * 4.0,
                    groundspeed: 14.0 + (t * 0.6).cos() as f32 * 3.0,
                    heading: (t * 10.0) as i16 % 360,
                    throttle: (50.0 + (t * 0.9).sin() as f32 * 20.0) as u16,
                    alt: 10.0,
                    climb: (t * 0.4).sin() as f32 * 2.0,
                });
                let _ = producer.send(enc(vfr));

                // 遥控通道（合成：1 横滚 / 2 俯仰 / 3 油门 / 4 偏航 / 5-8 开关）
                // 若 GCS 通过 RC_CHANNELS_OVERRIDE 覆盖（摇杆/键盘操控），优先回显覆盖值
                let ov = st.lock().await.rc_override;
                let base = [
                    (1500.0 + (t * 0.8).sin() * 300.0) as u16,
                    (1500.0 + (t * 0.6).cos() * 200.0) as u16,
                    (1400.0 + (t * 0.5).sin() * 300.0) as u16,
                    (1500.0 + (t * 0.4).sin() * 100.0) as u16,
                    1000u16,
                    1500u16,
                    1500u16,
                    1500u16,
                ];
                let rc = mlink::MavMessage::RC_CHANNELS(mav::RC_CHANNELS_DATA {
                    time_boot_ms: (t * 1000.0) as u32,
                    chan1_raw: if ov[0] > 0 { ov[0] } else { base[0] },
                    chan2_raw: if ov[1] > 0 { ov[1] } else { base[1] },
                    chan3_raw: if ov[2] > 0 { ov[2] } else { base[2] },
                    chan4_raw: if ov[3] > 0 { ov[3] } else { base[3] },
                    chan5_raw: base[4],
                    chan6_raw: base[5],
                    chan7_raw: base[6],
                    chan8_raw: base[7],
                    chan9_raw: 0,
                    chan10_raw: 0,
                    chan11_raw: 0,
                    chan12_raw: 0,
                    chan13_raw: 0,
                    chan14_raw: 0,
                    chan15_raw: 0,
                    chan16_raw: 0,
                    chan17_raw: 0,
                    chan18_raw: 0,
                    chancount: 8,
                    rssi: 190,
                });
                let _ = producer.send(enc(rc));

                // 围栏状态（无违例；点数通过 FENCE_POINT 上传/下载交互）
                let fst = mlink::MavMessage::FENCE_STATUS(mav::FENCE_STATUS_DATA {
                    breach_time: 0,
                    breach_count: 0,
                    breach_status: 0,
                    breach_type: mav::FenceBreach::FENCE_BREACH_NONE,
                });
                let _ = producer.send(enc(fst));
            }
        });

        Self {
            tx,
            rx: tokio::sync::Mutex::new(rx),
            stats: LinkStats::new(),
            header,
            state,
        }
    }

    /// 响应 GCS 下行：收到 PARAM_REQUEST_LIST 后回放示例参数；
    /// 收到 COMMAND_LONG 后回 COMMAND_ACK（模拟飞控执行）。
    async fn on_downlink(&self, bytes: &[u8]) {
        let parsed = mlink::read_v2_msg(bytes);
        if let Some((_h, msg, _)) = parsed {
            if let mav::MavMessage::PARAM_REQUEST_LIST(_) = &msg {
                // 回放一组示例参数
                let params: &[(&str, f32)] = &[
                    ("SYSID_THISMAV", 1.0),
                    ("RTL_ALT", 15.0),
                    ("FS_THR_ENABLE", 1.0),
                    ("WPNAV_SPEED", 500.0),
                    ("BATT_CAPACITY", 3300.0),
                ];
                let count = params.len() as u16;
                for (i, (name, val)) in params.iter().enumerate() {
                    let pv = mav::PARAM_VALUE_DATA {
                        param_id: string_to_cstr(name),
                        param_value: *val,
                        param_type: mav::MavParamType::MAV_PARAM_TYPE_REAL32,
                        param_count: count,
                        param_index: i as u16,
                    };
                    if let Ok(b) = mlink::encode_v2(&self.header, &mlink::MavMessage::PARAM_VALUE(pv))
                    {
                        let _ = self.tx.send(b);
                    }
                }
            } else if let mav::MavMessage::COMMAND_LONG(d) = &msg {
                // 模拟执行并回 ACK（任何指令都接受）
                let ack = mav::COMMAND_ACK_DATA {
                    command: d.command,
                    result: mav::MavResult::MAV_RESULT_ACCEPTED,
                    progress: 0,
                    result_param2: 0,
                    target_system: d.target_system,
                    target_component: d.target_component,
                };
                if let Ok(b) = mlink::encode_v2(&self.header, &mlink::MavMessage::COMMAND_ACK(ack))
                {
                    let _ = self.tx.send(b);
                }
            } else if let mav::MavMessage::RC_CHANNELS_OVERRIDE(d) = &msg {
                // 模拟飞控应用 RC 覆盖（摇杆/键盘操控），周期广播 RC_CHANNELS 时回显
                let mut st = self.state.lock().await;
                st.rc_override = [
                    d.chan1_raw, d.chan2_raw, d.chan3_raw, d.chan4_raw, d.chan5_raw,
                    d.chan6_raw, d.chan7_raw, d.chan8_raw,
                ];
            }
        }

        // FENCE_POINT / FENCE_FETCH_POINT 不在 common dialect，用原始帧解析
        if let Some(fp) = crate::mlink::fence::decode_fence_point(bytes) {
            // 飞控侧存储围栏点（按 idx 写入，count 用于扩容）
            let mut st = self.state.lock().await;
            if st.fence.len() <= fp.idx as usize {
                st.fence.resize(fp.idx as usize + 1, (0, 0));
            }
            st.fence[fp.idx as usize] = (fp.lat, fp.lng);
        } else if let Some(fq) = crate::mlink::fence::decode_fence_fetch_point(bytes) {
            // 模拟飞控回传 FENCE_POINT：idx 有效则回点，否则回 (0,0)
            let st = self.state.lock().await;
            let count = st.fence.len() as u8;
            let (lat, lng) = st
                .fence
                .get(fq.idx as usize)
                .copied()
                .unwrap_or((0, 0));
            drop(st);
            if let Ok(b) = crate::mlink::fence::encode_fence_point(
                &self.header,
                fq.target_system,
                fq.target_component,
                fq.idx,
                count,
                lat,
                lng,
            ) {
                let _ = self.tx.send(b);
            }
        }

        // MAVLink FTP（FILE_TRANSFER_PROTOCOL）：模拟飞控文件系统
        if let Some((tsys, tcomp, p)) = crate::mlink::ftp::decode_ftp(bytes) {
            let resp = self.handle_ftp(p).await;
            if let Ok(b) = crate::mlink::ftp::encode_ftp(&self.header, tsys, tcomp, &resp) {
                let _ = self.tx.send(b);
            }
        }
    }

    /// 处理一条 MAVLink FTP 请求，返回 ACK / NACK（模拟飞控侧实现）
    async fn handle_ftp(&self, p: crate::mlink::ftp::FtpPayload) -> crate::mlink::ftp::FtpPayload {
        use crate::mlink::ftp::*;
        let mut st = self.state.lock().await;
        match p.opcode {
            OP_CREATE_FILE => {
                let name = String::from_utf8_lossy(&p.data).to_string();
                if name.is_empty() {
                    return FtpPayload::nack(p.seq, 0, OP_CREATE_FILE, ERR_FAIL);
                }
                if st.ftp_files.contains_key(&name) {
                    return FtpPayload::nack(p.seq, 0, OP_CREATE_FILE, ERR_FILEEXISTS);
                }
                if st.ftp_sessions.len() >= 4 {
                    return FtpPayload::nack(p.seq, 0, OP_CREATE_FILE, ERR_NOSESSIONSAVAILABLE);
                }
                st.ftp_files.insert(name.clone(), Vec::new());
                let session = st.ftp_next_session;
                st.ftp_next_session = st.ftp_next_session.wrapping_add(1);
                st.ftp_sessions.push((
                    session,
                    FtpSimSession {
                        path: name,
                        offset: 0,
                    },
                ));
                FtpPayload::ack(p.seq, session, OP_CREATE_FILE, 0, Vec::new())
            }
            OP_WRITE_FILE => {
                if p.data.len() < 8 {
                    return FtpPayload::nack(p.seq, p.session, OP_WRITE_FILE, ERR_INVALIDDATASIZE);
                }
                let path = match st.ftp_sessions.iter().find(|(id, _)| *id == p.session) {
                    Some((_, s)) => s.path.clone(),
                    None => return FtpPayload::nack(p.seq, p.session, OP_WRITE_FILE, ERR_INVALIDSESSION),
                };
                let off = u64::from_le_bytes(p.data[..8].try_into().unwrap()) as usize;
                let content = &p.data[8..];
                let file = st.ftp_files.entry(path).or_default();
                if file.len() < off + content.len() {
                    file.resize(off + content.len(), 0);
                }
                file[off..off + content.len()].copy_from_slice(content);
                if let Some((_, s)) = st.ftp_sessions.iter_mut().find(|(id, _)| *id == p.session) {
                    s.offset = (off + content.len()) as u64;
                }
                FtpPayload::ack(p.seq, p.session, OP_WRITE_FILE, (off + content.len()) as u64, Vec::new())
            }
            OP_TERMINATE_SESSION => {
                st.ftp_sessions.retain(|(id, _)| *id != p.session);
                FtpPayload::ack(p.seq, p.session, OP_TERMINATE_SESSION, 0, Vec::new())
            }
            OP_RESET_SESSIONS => {
                st.ftp_sessions.clear();
                FtpPayload::ack(p.seq, 0, OP_RESET_SESSIONS, 0, Vec::new())
            }
            OP_OPEN_FILE_RO => {
                let name = String::from_utf8_lossy(&p.data).to_string();
                if !st.ftp_files.contains_key(&name) {
                    return FtpPayload::nack(p.seq, 0, OP_OPEN_FILE_RO, ERR_FILENOTFOUND);
                }
                if st.ftp_sessions.len() >= 4 {
                    return FtpPayload::nack(p.seq, 0, OP_OPEN_FILE_RO, ERR_NOSESSIONSAVAILABLE);
                }
                let session = st.ftp_next_session;
                st.ftp_next_session = st.ftp_next_session.wrapping_add(1);
                st.ftp_sessions.push((
                    session,
                    FtpSimSession {
                        path: name,
                        offset: 0,
                    },
                ));
                FtpPayload::ack(p.seq, session, OP_OPEN_FILE_RO, 0, Vec::new())
            }
            OP_READ_FILE => {
                let path = match st.ftp_sessions.iter().find(|(id, _)| *id == p.session) {
                    Some((_, s)) => s.path.clone(),
                    None => return FtpPayload::nack(p.seq, p.session, OP_READ_FILE, ERR_INVALIDSESSION),
                };
                let Some(file) = st.ftp_files.get(&path) else {
                    return FtpPayload::nack(p.seq, p.session, OP_READ_FILE, ERR_FILENOTFOUND);
                };
                let off = p.offset as usize;
                if off >= file.len() {
                    return FtpPayload::ack(p.seq, p.session, OP_READ_FILE, off as u64, Vec::new());
                }
                let end = (off + DATA_MAX).min(file.len());
                FtpPayload::ack(p.seq, p.session, OP_READ_FILE, end as u64, file[off..end].to_vec())
            }
            OP_LIST_DIRECTORY => {
                // 条目按文件名排序：每项 [类型 u8][文件名 NUL 结尾]，按 offset 分页
                let mut entries: Vec<u8> = Vec::new();
                let mut names: Vec<String> = st.ftp_files.keys().cloned().collect();
                names.sort();
                for n in &names {
                    entries.push(FILETYPE_FILE);
                    entries.extend_from_slice(n.as_bytes());
                    entries.push(0);
                }
                let off = p.offset as usize;
                if off >= entries.len() {
                    return FtpPayload::ack(p.seq, 0, OP_LIST_DIRECTORY, off as u64, Vec::new());
                }
                let end = (off + DATA_MAX).min(entries.len());
                FtpPayload::ack(p.seq, 0, OP_LIST_DIRECTORY, end as u64, entries[off..end].to_vec())
            }
            OP_CALC_FILE_CRC32 => {
                let name = String::from_utf8_lossy(&p.data).to_string();
                match st.ftp_files.get(&name) {
                    Some(f) => {
                        let crc = crate::mlink::ftp::crc32_ieee(f);
                        FtpPayload::ack(p.seq, 0, OP_CALC_FILE_CRC32, 0, crc.to_le_bytes().to_vec())
                    }
                    None => FtpPayload::nack(p.seq, 0, OP_CALC_FILE_CRC32, ERR_FILENOTFOUND),
                }
            }
            OP_REMOVE_FILE => {
                let name = String::from_utf8_lossy(&p.data).to_string();
                match st.ftp_files.remove(&name) {
                    Some(_) => FtpPayload::ack(p.seq, 0, OP_REMOVE_FILE, 0, Vec::new()),
                    None => FtpPayload::nack(p.seq, 0, OP_REMOVE_FILE, ERR_FILENOTFOUND),
                }
            }
            _ => FtpPayload::nack(p.seq, 0, p.opcode, ERR_UNKNOWNCOMMAND),
        }
    }
}

impl Default for SimLink {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Link for SimLink {
    async fn send(&self, bytes: &[u8]) -> Result<()> {
        // 模拟链路：把下行交给内部逻辑（如参数请求应答）
        self.on_downlink(bytes).await;
        Ok(())
    }

    async fn recv(&self) -> Result<Vec<u8>> {
        let mut rx = self.rx.lock().await;
        match rx.recv().await {
            Some(b) => {
                self.stats.add_recv(b.len() as u64).await;
                Ok(b)
            }
            None => Err(GcError::Link("sim closed".into())),
        }
    }

    fn quality(&self) -> LinkQuality {
        LinkQuality {
            signal_pct: 100,
            ..LinkQuality::default()
        }
    }

    fn is_open(&self) -> bool {
        true
    }

    fn name(&self) -> String {
        "sim".into()
    }
}
