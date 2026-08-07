//! 应用设置持久化：串口 / UDP / 地图 / 窗口偏好，存为本地 JSON 文件。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 应用偏好设置，保存/加载到本地配置文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// 默认串口名
    pub serial_port: String,
    /// 默认波特率
    pub baud: u32,
    /// UDP 绑定地址
    pub udp_bind: String,
    /// UDP 目标地址
    pub udp_target: String,
    /// 离线瓦片根目录（绝对路径），空 = 未设置
    pub tile_dir: Option<PathBuf>,
    /// 默认地图缩放
    pub map_zoom: f64,
    /// 窗口内宽
    pub window_w: f32,
    /// 窗口内高
    pub window_h: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            serial_port: "COM3".to_string(),
            baud: 57600,
            udp_bind: "0.0.0.0:14550".to_string(),
            udp_target: "127.0.0.1:14550".to_string(),
            tile_dir: None,
            map_zoom: 14.0,
            window_w: 1000.0,
            window_h: 680.0,
        }
    }
}

impl AppSettings {
    /// 返回配置文件路径：<用户配置目录>/groundctrl/settings.json
    pub fn path() -> Option<PathBuf> {
        let mut dir = dirs_if_available()?;
        dir.push("groundctrl");
        std::fs::create_dir_all(&dir).ok()?;
        dir.push("settings.json");
        Some(dir)
    }

    /// 加载设置；文件不存在或解析失败时返回默认值。
    pub fn load() -> Self {
        if let Some(p) = Self::path() {
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Ok(cfg) = serde_json::from_str::<AppSettings>(&s) {
                    return cfg;
                }
            }
        }
        Self::default()
    }

    /// 保存设置到磁盘；失败仅记录（不崩溃）。
    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Ok(json) = serde_json::to_string_pretty(self) {
                if let Err(e) = std::fs::write(&p, json) {
                    tracing::warn!("保存设置失败: {e}");
                }
            }
        }
    }
}

/// 跨平台用户配置目录（优先使用 dirs，回退到当前目录）。
fn dirs_if_available() -> Option<PathBuf> {
    // 避免引入 dirs crate：使用标准环境变量回退
    if let Some(p) = std::env::var_os("APPDATA") {
        // Windows: %APPDATA%
        return Some(PathBuf::from(p));
    }
    if let Some(p) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(p));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".config");
        return Some(p);
    }
    Some(PathBuf::from("."))
}
