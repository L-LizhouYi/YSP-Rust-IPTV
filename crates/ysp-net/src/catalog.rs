//! 频道目录客户端（磁盘缓存 + 12 小时 TTL）。

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use ysp_core::{
    config,
    models::{assign_ids, Channel},
    protocol::channels::parse_channels,
};

use crate::{
    error::NetError,
    http::{header, HttpClient},
};

pub struct CatalogClient {
    http: HttpClient,
    cache_dir: PathBuf,
    ttl: Duration,
}

impl CatalogClient {
    pub fn new(http: HttpClient, cache_dir: impl Into<PathBuf>, ttl: Duration) -> Self {
        Self {
            http,
            cache_dir: cache_dir.into(),
            ttl,
        }
    }

    fn cache_file(&self) -> PathBuf {
        self.cache_dir.join("channels.json")
    }

    async fn read_cache(&self) -> Option<Vec<Channel>> {
        let path = self.cache_file();
        let meta = tokio::fs::metadata(&path).await.ok()?;
        let age = SystemTime::now()
            .duration_since(meta.modified().ok()?)
            .unwrap_or(Duration::ZERO);
        if age > self.ttl {
            return None;
        }
        let text = tokio::fs::read_to_string(&path).await.ok()?;
        let mut channels: Vec<Channel> = serde_json::from_str(&text).ok()?;
        if channels.is_empty() {
            return None;
        }
        assign_ids(&mut channels);
        Some(channels)
    }

    async fn write_cache(&self, channels: &[Channel]) {
        let path = self.cache_file();
        if let Some(parent) = path.parent() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                tracing::warn!("创建缓存目录失败 {}: {err}", parent.display());
                return;
            }
        }
        let Ok(text) = serde_json::to_string_pretty(channels) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if tokio::fs::write(&tmp, text).await.is_ok() {
            let _ = tokio::fs::rename(&tmp, &path).await;
        }
    }

    pub async fn load(&self, force: bool) -> Result<Vec<Channel>, NetError> {
        if !force {
            if let Some(cached) = self.read_cache().await {
                return Ok(cached);
            }
        }
        let body = self
            .http
            .get(
                config::CHANNEL_PAGE_URL,
                &[
                    header("User-Agent", config::USER_AGENT),
                    header("Referer", config::REFERER),
                ],
                config::PAGE_MAX_BYTES,
            )
            .await?;
        let channels = parse_channels(&body);
        if channels.is_empty() {
            return Err(NetError::Parse("频道目录解析为空".into()));
        }
        self.write_cache(&channels).await;
        Ok(channels)
    }

    /// 按 livepid / id / 名称精确匹配，最后退化为名称包含。
    pub fn find<'a>(channels: &'a [Channel], ident: &str) -> Option<&'a Channel> {
        let ident = ident.trim();
        let lowered = ident.to_lowercase();
        channels
            .iter()
            .find(|c| c.livepid == ident || c.id() == lowered)
            .or_else(|| {
                channels
                    .iter()
                    .find(|c| c.name.to_lowercase() == lowered)
            })
            .or_else(|| {
                channels
                    .iter()
                    .find(|c| c.name.to_lowercase().contains(&lowered))
            })
    }
}
