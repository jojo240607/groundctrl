//! 统一错误类型

use thiserror::Error;

#[derive(Debug, Error)]
pub enum GcError {
    #[error("link error: {0}")]
    Link(String),

    #[error("mavlink parse error: {0}")]
    Mavlink(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("channel closed")]
    ChannelClosed,

    #[error("timeout")]
    Timeout,

    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("script error: {0}")]
    Script(String),
}

pub type Result<T> = std::result::Result<T, GcError>;
