//! 运行期配置（环境变量 + CLI）。

use std::path::PathBuf;
use std::time::Duration;

use ysp_core::{config, media::DecryptMode};

#[derive(Debug, Clone)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub cache_dir: PathBuf,
    pub guid: String,
    /// playurl 后台刷新间隔
    pub refresh: Duration,
    /// 全站分片缓存内存预算（字节）
    pub segment_mem_limit: usize,
    /// 单频道分片缓存预算（字节）
    pub channel_mem_limit: usize,
    /// 对外地址（反代/NAT 后面需要显式指定），为空时用请求 Host
    pub public_base: Option<String>,
    /// 强制解密模式（调试用），为空时按响应里的 encrypt 字段
    pub decrypt_override: Option<DecryptMode>,
    pub playlist_ttl: Duration,
    pub refresh_workers: usize,
}

impl Settings {
    pub fn from_env() -> Self {
        let cache_dir = std::env::var("YSP_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(".cache"));
        let guid = std::env::var("YSP_GUID")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| config::DEVICE_ID.to_string());
        let public_base = std::env::var("YSP_PUBLIC_BASE")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty());
        let decrypt_override = std::env::var("YSP_DECRYPT")
            .ok()
            .and_then(|s| DecryptMode::parse(&s));

        let segment_mb = env_usize("YSP_SEGMENT_MEM_MB", 256);
        let channel_mb = env_usize("YSP_CHANNEL_MEM_MB", 32);

        Self {
            host: "0.0.0.0".into(),
            port: 8525,
            cache_dir,
            guid,
            refresh: Duration::from_secs(config::PLAYURL_REFRESH_SECS),
            segment_mem_limit: segment_mb.max(8) * 1024 * 1024,
            channel_mem_limit: channel_mb.max(4) * 1024 * 1024,
            public_base,
            decrypt_override,
            playlist_ttl: Duration::from_millis(1500),
            refresh_workers: 4,
        }
    }

    /// 让单频道预算不超过全局预算的 1/2，避免一个频道吃满。
    pub fn normalized(mut self) -> Self {
        let half = (self.segment_mem_limit / 2).max(4 * 1024 * 1024);
        if self.channel_mem_limit > half {
            self.channel_mem_limit = half;
        }
        self
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}
