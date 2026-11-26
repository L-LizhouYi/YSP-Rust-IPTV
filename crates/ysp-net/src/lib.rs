//! 上游接口层：只负责「取数据」，不做缓存与并发编排。

pub mod catalog;
pub mod epg;
pub mod error;
pub mod http;
pub mod player;
pub mod signer;
pub mod token;

pub use error::NetError;
pub use http::HttpClient;
pub use player::Player;
