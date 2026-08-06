//! groundctrl-core: 跨平台飞控地面站核心库
//!
//! 仅依赖纯 Rust 库，不引用任何平台 API。

pub mod error;
pub mod link;
pub mod mlink;
pub mod proto;
pub mod services;
pub mod vehicle;

pub use error::{GcError, Result};
