//! UI 共享状态：主循环与各面板之间传递的本地状态。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use groundctrl_core::services::alarms::Alarm;
use groundctrl_core::vehicle::mission::Waypoint;
use groundctrl_core::vehicle::params::ParamEntry;
use groundctrl_core::vehicle::VehicleModel;

/// UI 共享状态
#[derive(Default)]
pub struct UiState {
    /// 当前选中飞机的遥测快照（最新一帧）
    pub vehicle: VehicleModel,
    /// 机队中所有飞机（按 system_id 聚合）
    pub vehicles: HashMap<u8, VehicleModel>,
    /// 当前选中查看的飞机 system_id（None = 自动选最新）
    pub selected_sys: Option<u8>,
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
    /// GPS 轨迹历史（按 system_id 分别记录经纬度）
    pub trails: HashMap<u8, Vec<(f64, f64)>>,
    /// 日志帧数
    pub log_frames: usize,
    /// 本地航点编辑
    pub mission: Vec<Waypoint>,
    pub wp_lat: f64,
    pub wp_lon: f64,
    pub wp_alt: f32,
    /// 地图缩放级别（越大越近）
    pub map_zoom: f64,
    /// 地图点击添加航点模式
    pub map_click_add_wp: bool,
    /// 用户指定的离线瓦片根目录（{z}/{x}/{y}.png）。None = 用默认在线缓存目录
    pub tile_dir: Option<PathBuf>,
    /// 是否启用在线瓦片下载
    pub online_tiles: bool,
    /// 在线瓦片源 URL 模板
    pub tile_url: String,
    /// 瓦片纹理缓存（key = "z/x/y"）
    pub tile_cache: HashMap<String, egui::TextureHandle>,
    /// 正在后台下载的瓦片 key 集合（避免重复触发）
    pub pending_tiles: HashSet<String>,
    /// 导出 / 导入反馈信息
    pub export_msg: String,
    pub import_msg: String,
    /// 参数实时趋势（参数名 -> (时间戳秒, 值) 序列），用于趋势图
    pub param_trends: HashMap<String, Vec<(f64, f64)>>,
    /// 是否启用参数趋势采样（1Hz）
    pub trend_enabled: bool,
    /// 趋势图当前选中的参数名（可叠加显示）
    pub trend_selected: Vec<String>,
    /// 地图中正在拖拽的航点索引
    pub dragging_wp: Option<usize>,
}

impl UiState {
    /// 实际瓦片根目录：用户目录优先，否则默认在线缓存目录（开箱即用）。
    pub fn tile_root(&self) -> Option<PathBuf> {
        if let Some(d) = &self.tile_dir {
            Some(d.clone())
        } else {
            crate::widgets::tiles::default_tile_cache_dir()
        }
    }

    /// 当前选中飞机的 GPS 轨迹（按 selected_sys；未指定则取任意一架）。
    pub fn active_trail(&self) -> Vec<(f64, f64)> {
        if let Some(s) = self.selected_sys.and_then(|s| self.trails.get(&s)) {
            s.clone()
        } else {
            self.trails.values().next().cloned().unwrap_or_default()
        }
    }
}
