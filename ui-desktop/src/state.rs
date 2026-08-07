//! UI 共享状态：主循环与各面板之间传递的本地状态。

use std::collections::HashMap;
use std::path::PathBuf;

use groundctrl_core::services::alarms::Alarm;
use groundctrl_core::vehicle::mission::Waypoint;
use groundctrl_core::vehicle::params::ParamEntry;
use groundctrl_core::vehicle::VehicleModel;

/// UI 共享状态
#[derive(Default)]
pub struct UiState {
    /// 当前遥测快照（最新一架飞机）
    pub vehicle: VehicleModel,
    pub log: Vec<String>,
    pub link_status: String,
    /// 当前活动链路名称（None = 未连接）
    pub active_link: Option<String>,
    /// 连接配置
    pub serial_port: String,
    pub baud: u32,
    pub udp_bind: String,
    pub udp_target: String,
    /// 当前标签页
    pub tab: crate::app::TabKind,
    /// 参数缓存
    pub params: Vec<ParamEntry>,
    pub params_complete: bool,
    pub params_received: u16,
    pub params_expected: u16,
    /// 告警（累积，带时间戳）
    pub alarms: Vec<(u64, Alarm)>,
    /// GPS 轨迹历史（经纬度）
    pub trail: Vec<(f64, f64)>,
    /// 日志帧数
    pub log_frames: usize,
    /// 本地航点编辑
    pub mission: Vec<Waypoint>,
    pub wp_lat: f64,
    pub wp_lon: f64,
    pub wp_alt: f32,
    /// 地图缩放级别（越大越近）
    pub map_zoom: f64,
    /// 离线瓦片根目录（{z}/{x}/{y}.png），None = 退化 HUD
    pub tile_dir: Option<PathBuf>,
    /// 瓦片纹理缓存（key = "z/x/y"）
    pub tile_cache: HashMap<String, egui::TextureHandle>,
    /// 导出 / 导入反馈信息
    pub export_msg: String,
    pub import_msg: String,
}
