//! 央视频协议/解密核心。
//!
//! 依赖方向：`media -> crypto`，全部是纯函数，不做任何 IO，
//! 因此可以直接被单元测试覆盖，也方便以后替换上层实现。

pub mod config;
pub mod crypto;
pub mod error;
pub mod media;
pub mod models;
pub mod protocol;
pub mod util;

pub use error::YspError;
