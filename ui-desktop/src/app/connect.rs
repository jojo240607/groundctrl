//! 标签页种类与链路连接类型。

/// 面板标签页
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabKind {
    #[default]
    Telemetry,
    Params,
    Mission,
    Map,
    Log,
    Alarms,
    Settings,
}

/// 链路连接种类
pub enum ConnectKind {
    Sim,
    Udp(String, String),
    Serial(String, u32),
}
