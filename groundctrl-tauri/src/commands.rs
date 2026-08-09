//! Tauri 命令：前端通过 invoke 调用的后端接口。

use crate::frontend::*;
use crate::state::*;
use groundctrl_core::link;
use tauri::{AppHandle, Emitter, State};

/// 连接类型
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectKindJson {
    Sim,
    Udp,
    Serial,
}

/// 连接命令参数
#[derive(Debug, serde::Deserialize)]
pub struct ConnectArgs {
    pub kind: ConnectKindJson,
    /// UDP: 绑定地址；Serial: 端口名
    pub bind: String,
    /// UDP: 目标地址；Serial: 波特率
    pub target: String,
}

/// 建立链路连接。
#[tauri::command]
pub async fn connect(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ConnectArgs,
) -> Result<(), String> {
    let hub = state.hub.clone();
    let link_handle: link::LinkHandle = match args.kind {
        ConnectKindJson::Sim => link::share(link::sim::SimLink::new()),
        ConnectKindJson::Udp => {
            match link::udp::UdpLink::new(link::udp::UdpConfig {
                bind_addr: args.bind.clone(),
                target_addr: args.target.clone(),
            })
            .await
            {
                Ok(l) => link::share(l),
                Err(e) => return Err(format!("UDP 打开失败: {e}")),
            }
        }
        ConnectKindJson::Serial => {
            let baud: u32 = args.target.parse().unwrap_or(57600);
            link::share(link::serial::SerialLink::new(link::serial::SerialConfig {
                port: args.bind.clone(),
                baud_rate: baud,
            }))
        }
    };
    // 保存连接 URL 供后续参考
    {
        let mut u = state.connect_url.lock().unwrap();
        *u = format!("{:?}|{}|{}", args.kind, args.bind, args.target);
    }
    hub.connect(link_handle).await;
    let _ = app.emit("link-state", &LinkStateJson { link: "user".into(), connected: true });
    Ok(())
}

/// 断开当前链路。
#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<(), String> {
    state.hub.disconnect().await;
    Ok(())
}

/// 枚举当前系统可用串口（如 Windows 的 COM3、COM9），供前端串口连接下拉选择。
/// USB CDC-ACM 虚拟串口插上后即出现在此列表中。
#[tauri::command]
pub fn list_serial_ports() -> Vec<String> {
    match serialport::available_ports() {
        Ok(ports) => ports.into_iter().map(|p| p.port_name).collect(),
        Err(e) => {
            tracing::warn!("enumerate serial ports failed: {e}");
            Vec::new()
        }
    }
}

/// 主动拉取当前机队快照。
#[tauri::command]
pub async fn get_fleet(state: State<'_, AppState>) -> Result<FleetSnapshot, String> {
    let fleet = crate::subscribe::build_fleet(&state.hub).await;
    Ok(fleet)
}

/// 读取设置。
#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> SettingsJson {
    crate::state::load_settings(state.inner())
}

/// 保存设置。
#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, s: SettingsJson) -> Result<(), String> {
    crate::state::save_settings(state.inner(), &s);
    Ok(())
}

/// 读取告警规则。
#[tauri::command]
pub async fn get_monitor_config(state: State<'_, AppState>) -> Result<MonitorConfigJson, String> {
    let cfg = state.hub.monitor_config().await;
    Ok(cfg.into())
}

/// 写入告警规则。
#[tauri::command]
pub async fn set_monitor_config(
    state: State<'_, AppState>,
    cfg: MonitorConfigJson,
) -> Result<(), String> {
    state.hub.set_monitor_config(cfg.into()).await;
    Ok(())
}

/// 请求飞控上报全部参数。
#[tauri::command]
pub async fn request_params(
    state: State<'_, AppState>,
    sys: u8,
    comp: u8,
) -> Result<(), String> {
    state
        .hub
        .request_params(sys, comp)
        .await
        .map_err(|e| e.to_string())
}

/// 写回单个参数。
#[tauri::command]
pub async fn set_param(
    state: State<'_, AppState>,
    sys: u8,
    comp: u8,
    name: String,
    value: f32,
) -> Result<(), String> {
    state
        .hub
        .set_param(sys, comp, &name, value)
        .await
        .map_err(|e| e.to_string())
}

/// 上传航点。
#[tauri::command]
pub async fn upload_mission(
    state: State<'_, AppState>,
    sys: u8,
    comp: u8,
    items: Vec<WaypointItem>,
) -> Result<(), String> {
    let wps: Vec<groundctrl_core::vehicle::mission::Waypoint> = items
        .into_iter()
        .enumerate()
        .map(|(i, it)| groundctrl_core::vehicle::mission::Waypoint {
            seq: i as u16,
            frame: groundctrl_core::vehicle::params::common::MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT as u8,
            command: it.command,
            param1: 0.0,
            param2: 0.0,
            param3: 0.0,
            param4: 0.0,
            lat: it.x,
            lon: it.y,
            alt: it.z,
            autocontinue: if it.autocontinue { 1 } else { 0 },
        })
        .collect();
    state
        .hub
        .upload_mission(sys, comp, &wps)
        .await
        .map_err(|e| e.to_string())
}

/// 下载航点：返回地面站侧缓存的航点（最近一次编辑/上传的航点）。
#[tauri::command]
pub async fn download_mission(
    state: State<'_, AppState>,
    _sys: u8,
    _comp: u8,
) -> Result<Vec<WaypointItem>, String> {
    let wps = state.hub.get_mission().await;
    Ok(wps
        .into_iter()
        .map(|wp| WaypointItem {
            seq: wp.seq,
            command: wp.command,
            x: wp.lat,
            y: wp.lon,
            z: wp.alt,
            autocontinue: wp.autocontinue != 0,
        })
        .collect())
}
