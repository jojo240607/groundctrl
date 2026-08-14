//! 国际化（P3-3）：zh-CN / en-US 语言字典。
//!
//! 采用「中文即 key」的轻量方案：调用 `tr(lang, "中文文本")`，
//! 英文模式下命中字典返回译文，否则回退中文原文（新文本无需强制补译）。

use serde::{Deserialize, Serialize};

/// 界面语言
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Lang {
    #[default]
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
}

impl Lang {
    /// 语言显示名（用于设置面板下拉框）
    pub fn label(self) -> &'static str {
        match self {
            Lang::ZhCn => "简体中文",
            Lang::EnUs => "English",
        }
    }

    /// 查询翻译；英文未收录时回退中文原文
    pub fn tr<'a>(self, zh: &'a str) -> &'a str {
        match self {
            Lang::ZhCn => zh,
            Lang::EnUs => en(zh).unwrap_or(zh),
        }
    }
}

/// 中文 -> 英文 字典（未列出的文本保持中文）
fn en(zh: &str) -> Option<&'static str> {
    Some(match zh {
        // ---- 标签页 / 主框架 ----
        "遥测" => "Telemetry",
        "参数" => "Params",
        "航点" => "Mission",
        "地图" => "Map",
        "日志" => "Log",
        "告警" => "Alarms",
        "趋势" => "Trends",
        "校准" => "Calibration",
        "操控" => "Joystick",
        "固件" => "Firmware",
        "视频" => "Video",
        "脚本" => "Scripts",
        "设置" => "Settings",

        // ---- 连接面板 ----
        "连接" => "Connection",
        "状态:" => "Status:",
        "断开当前链路" => "Disconnect",
        "串口" => "Serial",
        "连接串口" => "Connect Serial",
        "连接 UDP" => "Connect UDP",
        "模拟链路 (Sim)" => "Sim Link",

        // ---- 遥测面板 ----
        "模式:" => "Mode:",
        "姿态" => "Attitude",
        "飞行控制" => "Flight Control",
        "解锁" => "Arm",
        "上锁" => "Disarm",
        "起飞" => "Takeoff",
        "降落" => "Land",
        "返航" => "RTL",
        "油门" => "Throttle",
        "电池" => "Battery",
        "GPS" => "GPS",
        "回执:" => "ACK:",
        "切换" => "Switch",
        "遥控" => "RC",
        "心跳" => "Heartbeat",

        // ---- 校准面板 ----
        "传感器校准向导" => "Sensor Calibration Wizard",
        "开始校准" => "Start Calibration",
        "校准进行中..." => "Calibration in progress...",

        // ---- 固件面板 ----
        "固件升级（MAVLink FTP）" => "Firmware Upgrade (MAVLink FTP)",
        "选择固件文件..." => "Choose Firmware File...",
        "未选择文件" => "No file selected",
        "上传到飞控" => "Upload to FC",
        "下载到本地..." => "Download...",
        "计算 CRC32" => "Calc CRC32",
        "删除文件" => "Delete",
        "飞控存储" => "FC Storage",
        "刷新列表" => "Refresh",
        "文件" => "File",
        "目录" => "Dir",
        "（空）" => "(empty)",

        // ---- 视频面板 ----
        "视频 / OSD" => "Video / OSD",
        "监听地址:" => "Listen address:",
        "启动接收" => "Start",
        "停止接收" => "Stop",
        "启动模拟视频源" => "Start Sim Video Source",
        "停止模拟源" => "Stop Sim Source",
        "OSD 叠加" => "OSD Overlay",

        // ---- 脚本面板 ----
        "脚本任务" => "Script Tasks",
        "任务名称:" => "Task name:",
        "载入示例" => "Load Sample",
        "运行" => "Run",
        "停止" => "Stop",
        "脚本内容:" => "Script:",
        "运行日志" => "Run Log",

        // ---- 设置面板 ----
        "语言" => "Language",
        "保存到磁盘" => "Save to Disk",
        "重置为默认" => "Reset to Defaults",
        "连接默认值" => "Connection Defaults",
        "地图 / 瓦片" => "Map / Tiles",
        "声音告警" => "Sound Alerts",
        "告警规则（电量 / 地理围栏）" => "Alarm Rules (Battery / Geofence)",
        "波特率" => "Baud rate",
        "启用在线瓦片下载 (开箱即用)" => "Enable online tiles (works out of the box)",
        "瓦片源 URL:" => "Tile source URL:",
        "默认缩放" => "Default zoom",
        "清除离线目录" => "Clear offline dir",
        "选择目录..." => "Choose dir...",
        "当前标签页:" => "Current tab:",

        _ => return None,
    })
}
