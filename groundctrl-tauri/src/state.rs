//! 后端应用状态：持有 TelemetryHub、连接参数、设置与订阅句柄。

use groundctrl_core::services::TelemetryHub;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

use crate::frontend::SettingsJson;

/// 后端共享状态。
pub struct AppState {
    pub hub: TelemetryHub,
    /// 最近一次请求的连接 URL（由前端设置，用于重连）。
    pub connect_url: Mutex<String>,
    /// 设置文件路径（与 exe 同目录的 settings.json）。
    pub settings_path: PathBuf,
}

impl AppState {
    pub fn new(app: &AppHandle) -> Self {
        let hub = TelemetryHub::new();
        // 设置文件放在用户数据目录（跨平台稳定位置）。Tauri 提供可写目录。
        let settings_path = app
            .path()
            .app_config_dir()
            .map(|p| {
                std::fs::create_dir_all(&p).ok();
                p.join("settings.json")
            })
            .unwrap_or_else(|_| PathBuf::from("settings.json"));
        AppState {
            hub,
            connect_url: Mutex::new("tcp:127.0.0.1:5760".to_string()),
            settings_path,
        }
    }
}

/// 加载设置（不存在则用默认）。
pub fn load_settings(state: &AppState) -> SettingsJson {
    let path = &state.settings_path;
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(s) = serde_json::from_str::<SettingsJson>(&text) {
            return s;
        }
    }
    SettingsJson::default()
}

/// 保存设置。
pub fn save_settings(state: &AppState, s: &SettingsJson) {
    if let Ok(text) = serde_json::to_string_pretty(s) {
        let _ = std::fs::write(&state.settings_path, text);
    }
}
