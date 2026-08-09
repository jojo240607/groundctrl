//! Ground Control Station — Tauri 应用（Web/HTML 前端 + Rust 后端）。

mod commands;
mod frontend;
mod state;
mod subscribe;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let app_state = AppState::new(&handle);
            // 启动时应用已保存设置中的告警规则（可选）
            app.manage(app_state);
            // 启动遥测订阅与事件转发
            let st = app.state::<AppState>();
            subscribe::start_subscription(&handle, st.hub.clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::disconnect,
            commands::list_serial_ports,
            commands::get_fleet,
            commands::get_settings,
            commands::save_settings,
            commands::get_monitor_config,
            commands::set_monitor_config,
            commands::request_params,
            commands::set_param,
            commands::upload_mission,
            commands::download_mission,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
