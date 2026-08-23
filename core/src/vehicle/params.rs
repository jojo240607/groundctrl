//! 参数管理：读取 / 缓存 / 写回飞控参数
//!
//! 协议基于 MAVLink `common` dialect 的 PARAM 消息族：
//! - [`PARAM_REQUEST_LIST`]：请求飞控上报全部参数
//! - [`PARAM_VALUE`]：飞控回应的单个参数（含 id / 值 / 计数）
//! - [`PARAM_SET`]：地面站写回参数
//!
//! `ParamManager` 维护一个按参数名索引的缓存，并提供等待「全部到齐」的接口。

use std::collections::BTreeMap;

use mavlink_core::common as mav;

/// 单个参数缓存项
#[derive(Debug, Clone, PartialEq)]
pub struct ParamEntry {
    /// 参数名（MAVLink 协议内为 16 字节定长，截断为字符串）
    pub name: String,
    /// 参数值（统一按 f32 处理，MAVLink 协议本身用 float 表示）
    pub value: f32,
    /// 参数序号（PARAM_VALUE.param_index）
    pub index: u16,
}

/// 参数管理器：按飞机 system_id 缓存参数
#[derive(Debug, Default, Clone)]
pub struct ParamManager {
    /// 参数名 -> 缓存项
    params: BTreeMap<String, ParamEntry>,
    /// 期望的参数总数（来自 PARAM_VALUE.param_count），0 表示未知
    expected_count: u16,
    /// 最近一次完整拉取是否已完成
    complete: bool,
}

impl ParamManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 应用一条飞控回应的 PARAM_VALUE
    ///
    /// 返回是否刚刚集满全部参数（从「未完成」变为「完成」的瞬间为 true）。
    pub fn apply_param_value(&mut self, d: &mav::PARAM_VALUE_DATA) -> bool {
        let name = cstr_to_string(&d.param_id);
        let index = d.param_index;
        // 用 PARAM_VALUE 自带的计数作为期望值（若合法）
        if d.param_count > 0 {
            self.expected_count = d.param_count as u16;
        }
        let entry = ParamEntry {
            name: name.clone(),
            value: d.param_value,
            index,
        };
        let was_complete = self.complete;
        self.params.insert(name, entry);

        if self.expected_count > 0 && (self.params.len() as u16) >= self.expected_count {
            self.complete = true;
        }
        self.complete && !was_complete
    }

    /// 构造一条请求全部参数的消息
    pub fn make_request_list(target_sys: u8, target_comp: u8) -> mav::MavMessage {
        mav::MavMessage::PARAM_REQUEST_LIST(mav::PARAM_REQUEST_LIST_DATA {
            target_system: target_sys,
            target_component: target_comp,
        })
    }

    /// 构造一条读取单个参数的消息
    pub fn make_request_read(
        target_sys: u8,
        target_comp: u8,
        name: &str,
    ) -> mav::MavMessage {
        let param_id = string_to_cstr(name);
        mav::MavMessage::PARAM_REQUEST_READ(mav::PARAM_REQUEST_READ_DATA {
            target_system: target_sys,
            target_component: target_comp,
            param_id,
            param_index: -1, // 用名字查
        })
    }

    /// 构造一条写回参数的消息
    pub fn make_set(
        target_sys: u8,
        target_comp: u8,
        name: &str,
        value: f32,
    ) -> mav::MavMessage {
        let param_id = string_to_cstr(name);
        mav::MavMessage::PARAM_SET(mav::PARAM_SET_DATA {
            target_system: target_sys,
            target_component: target_comp,
            param_id,
            param_value: value,
            param_type: mav::MavParamType::MAV_PARAM_TYPE_REAL32,
        })
    }

    /// 当前缓存的所有参数（按名字排序）
    pub fn list(&self) -> Vec<ParamEntry> {
        self.params.values().cloned().collect()
    }

    /// 按名字取参数
    pub fn get(&self, name: &str) -> Option<&ParamEntry> {
        self.params.get(name)
    }

    /// 在缓存内就地更新一个参数值（写回后飞控会回 PARAM_VALUE 确认，
    /// 这里先乐观更新以便 UI 即时反馈；以飞控回执为准）。
    pub fn set_local(&mut self, name: &str, value: f32) {
        if let Some(e) = self.params.get_mut(name) {
            e.value = value;
        }
    }

    /// 是否已集满全部参数
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// 已收到的参数个数
    pub fn len(&self) -> usize {
        self.params.len()
    }

    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// 期望总数（未知时为 0）
    pub fn expected(&self) -> u16 {
        self.expected_count
    }

    /// 清空缓存（重新拉取前调用）
    pub fn clear(&mut self) {
        self.params.clear();
        self.expected_count = 0;
        self.complete = false;
    }
}

/// 把 MAVLink 定长 param_id（C 风格字符串）转成 Rust String
pub fn cstr_to_string(id: &[u8; 16]) -> String {
    let bytes: Vec<u8> = id
        .iter()
        .copied()
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// 把 param_id 字符串回填进 MAVLink 的 [u8; 16] 定长数组
pub fn string_to_cstr(name: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    let bytes = name.as_bytes();
    let n = bytes.len().min(16);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

// 让上层能引用 mavlink 的 common 别名（避免重复 use）
pub use mavlink_core::common as common;
