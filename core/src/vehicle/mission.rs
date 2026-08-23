//! 航点规划：航点增删改、上传 / 下载、FENCE、RALLY
//!
//! 协议基于 MAVLink `common` dialect 的 MISSION 消息族（简化实现，
//! 覆盖最常用的航点 `MAV_CMD_NAV_WAYPOINT` 与上传/下载握手）：
//! - [`MISSION_REQUEST_LIST`] / [`MISSION_COUNT`]：协商航点总数
//! - [`MISSION_REQUEST`] / [`MISSION_ITEM`]：逐条拉取 / 下发
//! - [`MISSION_ACK`]：飞控确认接收完成
//! - [`MISSION_CLEAR_ALL`]：清空飞控航点
//!
//! `MissionPlanner` 维护本地的航点列表，并负责把整份航点编码成 MISSION_ITEM 序列。

use mavlink_core::common as mav;

/// 单个航点（简化：仅导航航点需要的字段）
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Waypoint {
    pub seq: u16,
    pub frame: u8,       // MAV_FRAME
    pub command: u16,    // MAV_CMD
    pub param1: f32,
    pub param2: f32,
    pub param3: f32,
    pub param4: f32,
    pub lat: f64,        // deg
    pub lon: f64,        // deg
    pub alt: f32,        // m
    pub autocontinue: u8,
}

impl Default for Waypoint {
    fn default() -> Self {
        Self {
            seq: 0,
            frame: mav::MavFrame::MAV_FRAME_GLOBAL_RELATIVE_ALT as u8,
            command: mav::MavCmd::MAV_CMD_NAV_WAYPOINT as u16,
            param1: 0.0,
            param2: 0.0,
            param3: 0.0,
            param4: 0.0,
            lat: 0.0,
            lon: 0.0,
            alt: 0.0,
            autocontinue: 1,
        }
    }
}

impl Waypoint {
    /// 构造一个常规导航航点
    pub fn nav(lat: f64, lon: f64, alt: f32) -> Self {
        Self {
            lat,
            lon,
            alt,
            ..Default::default()
        }
    }

    /// 转成 MAVLink MISSION_ITEM_INT（精度更高的整型坐标版本）
    pub fn to_item_int(&self, target_sys: u8, target_comp: u8) -> mav::MavMessage {
        mav::MavMessage::MISSION_ITEM_INT(mav::MISSION_ITEM_INT_DATA {
            target_system: target_sys,
            target_component: target_comp,
            seq: self.seq,
            frame: unsafe { std::mem::transmute(self.frame) },
            command: unsafe { std::mem::transmute(self.command) },
            current: 0,
            autocontinue: self.autocontinue,
            param1: self.param1,
            param2: self.param2,
            param3: self.param3,
            param4: self.param4,
            x: (self.lat * 1e7) as i32,
            y: (self.lon * 1e7) as i32,
            z: self.alt,
            mission_type: 0,
        })
    }

    /// 从飞控回传的 MISSION_ITEM_INT_DATA 解析出航点
    pub fn from_item_int(d: &mav::MISSION_ITEM_INT_DATA) -> Self {
        Self {
            seq: d.seq,
            frame: d.frame as u8,
            command: d.command as u16,
            param1: d.param1,
            param2: d.param2,
            param3: d.param3,
            param4: d.param4,
            lat: d.x as f64 / 1e7,
            lon: d.y as f64 / 1e7,
            alt: d.z,
            autocontinue: d.autocontinue,
        }
    }
}

/// 航点管理器
#[derive(Debug, Default)]
pub struct MissionPlanner {
    /// 本地航点列表（按 seq 升序）
    items: Vec<Waypoint>,
    /// 上传/下载进行中的状态
    in_progress: bool,
}

impl MissionPlanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// 从航点切片构造（用于上传时生成 MISSION_COUNT）
    pub fn from_items(items: &[Waypoint]) -> Self {
        Self {
            items: items.to_vec(),
            in_progress: false,
        }
    }

    /// 当前航点列表
    pub fn items(&self) -> &[Waypoint] {
        &self.items
    }

    /// 航点数量
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 追加一个航点（自动分配 seq）
    pub fn push(&mut self, mut wp: Waypoint) {
        wp.seq = self.items.len() as u16;
        self.items.push(wp);
    }

    /// 在指定索引插入
    pub fn insert(&mut self, index: usize, wp: Waypoint) {
        self.items.insert(index, wp);
        self.renumber();
    }

    /// 删除指定索引
    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
            self.renumber();
        }
    }

    /// 修改指定航点
    pub fn update(&mut self, index: usize, wp: Waypoint) {
        if index < self.items.len() {
            let mut wp = wp;
            wp.seq = index as u16;
            self.items[index] = wp;
        }
    }

    /// 清空本地航点
    pub fn clear(&mut self) {
        self.items.clear();
    }

    fn renumber(&mut self) {
        for (i, wp) in self.items.iter_mut().enumerate() {
            wp.seq = i as u16;
        }
    }

    /// 生成本地航点对应的 MISSION_ITEM_INT 序列（用于上传）
    pub fn to_items(&self, target_sys: u8, target_comp: u8) -> Vec<mav::MavMessage> {
        self.items
            .iter()
            .map(|wp| wp.to_item_int(target_sys, target_comp))
            .collect()
    }

    /// 构造请求飞控航点总数的消息
    pub fn make_request_list(target_sys: u8, target_comp: u8) -> mav::MavMessage {
        mav::MavMessage::MISSION_REQUEST_LIST(mav::MISSION_REQUEST_LIST_DATA {
            target_system: target_sys,
            target_component: target_comp,
            mission_type: 0,
        })
    }

    /// 构造请求飞控回传单条航点的消息（按 seq）
    pub fn make_request(target_sys: u8, target_comp: u8, seq: u16) -> mav::MavMessage {
        mav::MavMessage::MISSION_REQUEST(mav::MISSION_REQUEST_DATA {
            target_system: target_sys,
            target_component: target_comp,
            mission_type: 0,
            seq,
        })
    }

    /// 从飞控回传的整型航点序列构造
    pub fn from_int_items(items: &[mav::MISSION_ITEM_INT_DATA]) -> Self {
        let mut planner = Self::new();
        for d in items {
            planner.items.push(Waypoint::from_item_int(d));
        }
        planner.renumber();
        planner
    }

    /// 构造清空飞控航点的消息
    pub fn make_clear_all(target_sys: u8, target_comp: u8) -> mav::MavMessage {
        mav::MavMessage::MISSION_CLEAR_ALL(mav::MISSION_CLEAR_ALL_DATA {
            target_system: target_sys,
            target_component: target_comp,
            mission_type: 0,
        })
    }

    /// 上传开始时，先发 MISSION_COUNT 告诉飞控有多少条
    pub fn make_count(&self, target_sys: u8, target_comp: u8) -> mav::MavMessage {
        mav::MavMessage::MISSION_COUNT(mav::MISSION_COUNT_DATA {
            target_system: target_sys,
            target_component: target_comp,
            count: self.items.len() as u16,
            mission_type: 0,
        })
    }

    pub fn mark_in_progress(&mut self, v: bool) {
        self.in_progress = v;
    }

    pub fn in_progress(&self) -> bool {
        self.in_progress
    }
}
