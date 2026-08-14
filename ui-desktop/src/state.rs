//! UI 共享状态：主循环与各面板之间传递的本地状态。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use groundctrl_core::services::alarms::{Alarm, MonitorConfig};
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
    /// 界面语言（P3-3）
    pub lang: crate::i18n::Lang,
    /// 参数缓存
    pub params: Vec<ParamEntry>,
    pub params_complete: bool,
    pub params_received: u16,
    pub params_expected: u16,
    /// 告警（累积，带时间戳）
    pub alarms: Vec<(u64, Alarm)>,
    /// 声音告警开关（P2-4）
    pub sound_enabled: bool,
    /// 已播放提示音的告警条数（去重）
    pub sound_alarm_played: usize,
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
    /// tlog 分析提取的序列（P2-1 日志图表分析）
    pub log_series: Vec<groundctrl_core::services::log::LogSeries>,
    /// tlog 分析当前选中的序列索引
    pub log_series_sel: Vec<usize>,
    /// tlog 分析反馈信息
    pub log_series_msg: String,
    /// 校准指令发送中（防重复点击）
    pub cal_busy: bool,
    /// 校准状态信息
    pub cal_msg: String,
    /// 键盘操控开关（P2-3）
    pub ctrl_kb_enabled: bool,
    /// 摇杆操控开关（P2-3）
    pub ctrl_js_enabled: bool,
    /// 上次发送 RC 覆盖的时间（egui 秒，节流 10Hz）
    pub ctrl_last_send: f64,
    /// 最近一次发送的通道值（用于面板预览）
    pub ctrl_last_chans: [u16; 8],
    /// 手柄名称（空 = 未连接）
    pub joystick_name: String,
    /// 手柄轴原始值（LX/LY/RX/RY/LT2/RT2，-1..1）
    pub joystick_axes: [f32; 6],
    /// 地图中正在拖拽的航点索引
    pub dragging_wp: Option<usize>,
    /// 告警监控阈值（用户在设置面板编辑，保存到 core 的 FlightMonitor）
    pub monitor_cfg: MonitorConfig,
    /// 地理围栏编辑（经纬度列表）
    pub fence: Vec<(f64, f64)>,
    /// 围栏操作反馈信息
    pub fence_msg: String,
    /// 围栏编辑器：待添加点的纬度/经度
    pub fence_lat: f64,
    pub fence_lon: f64,
    /// 固件升级：已选择的本地固件（名称, 内容）
    pub fw_file: Option<(String, Vec<u8>)>,
    /// 固件操作进行中（防重复点击）
    pub fw_busy: bool,
    /// 上传进度 0..1
    pub fw_progress: f32,
    /// 固件操作反馈信息
    pub fw_msg: String,
    /// 飞控侧 FTP 文件列表（类型, 名称）
    pub fw_files: Vec<(u8, String)>,
    /// 飞控文件列表中选中的文件
    pub fw_sel_name: String,
    /// 视频：UDP 绑定地址（图传流监听地址）
    pub video_addr: String,
    /// 视频接收运行中
    pub video_running: bool,
    /// OSD 叠加开关
    pub video_osd: bool,
    /// 模拟视频源运行中
    pub video_sim: bool,
    /// 视频统计（帧数, 字节数）
    pub video_stats: (u64, u64),
    /// 视频反馈信息
    pub video_msg: String,
    /// 最近解码的视频帧纹理
    pub video_texture: Option<egui::TextureHandle>,
    /// 已绘制纹理对应的帧序号（避免重复解码）
    pub video_drawn_seq: u64,
    /// 脚本任务：编辑器文本（P3-4）
    pub script_text: String,
    /// 脚本任务：任务名称
    pub script_name: String,
    /// 脚本任务：运行日志
    pub script_log: Vec<String>,
    /// 脚本任务：运行中
    pub script_running: bool,
    /// 脚本任务：反馈信息
    pub script_msg: String,
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
