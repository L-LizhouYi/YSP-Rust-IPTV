//! EPG（节目单）客户端：内存 LRU + 磁盘缓存（5 分钟 TTL）。

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use lru::LruCache;
use tokio::sync::Mutex;
use ysp_core::{
    config,
    models::Program,
    protocol::epg::{current_program, parse_epg},
    util::{now_secs, today_compact},
};

use crate::{
    error::NetError,
    http::{header, HttpClient},
};

pub struct EpgClient {
    http: HttpClient,
    cache_dir: PathBuf,
    ttl: Duration,
    mem: Mutex<LruCache<String, (Instant, Arc<Vec<Program>>)>>,
}

impl EpgClient {
    pub fn new(http: HttpClient, cache_dir: impl Into<PathBuf>, ttl: Duration) -> Self {
        Self {
            http,
            cache_dir: cache_dir.into(),
            ttl,
            mem: Mutex::new(LruCache::new(NonZeroUsize::new(128).unwrap())),
        }
    }

    fn cache_file(&self, livepid: &str, date: &str) -> PathBuf {
        self.cache_dir.join("epg").join(format!("{livepid}-{date}.json"))
    }

    async fn read_disk(&self, livepid: &str, date: &str) -> Option<Vec<Program>> {
        let path = self.cache_file(livepid, date);
        let meta = tokio::fs::metadata(&path).await.ok()?;
        let age = SystemTime::now()
            .duration_since(meta.modified().ok()?)
            .unwrap_or(Duration::ZERO);
        if age > self.ttl {
            return None;
        }
        let text = tokio::fs::read_to_string(&path).await.ok()?;
        serde_json::from_str(&text).ok()
    }

    async fn write_disk(&self, livepid: &str, date: &str, programs: &[Program]) {
        let path = self.cache_file(livepid, date);
        if let Some(parent) = path.parent() {
            if tokio::fs::create_dir_all(parent).await.is_err() {
                return;
            }
        }
        if let Ok(text) = serde_json::to_string(programs) {
            let _ = tokio::fs::write(&path, text).await;
        }
    }

    pub async fn programs(
        &self,
        livepid: &str,
        date: &str,
    ) -> Result<Arc<Vec<Program>>, NetError> {
        let date = if date.is_empty() {
            today_compact()
        } else {
            date.to_string()
        };
        let key = format!("{livepid}-{date}");

        {
            let mut mem = self.mem.lock().await;
            if let Some((at, programs)) = mem.get(&key) {
                if at.elapsed() < self.ttl {
                    return Ok(programs.clone());
                }
            }
        }
        if let Some(programs) = self.read_disk(livepid, &date).await {
            let programs = Arc::new(programs);
            self.mem
                .lock()
                .await
                .put(key, (Instant::now(), programs.clone()));
            return Ok(programs);
        }

        let url = format!("{}/{}/{}", config::EPG_URL_PREFIX, livepid, date);
        let body = self
            .http
            .get(
                &url,
                &[
                    header("User-Agent", config::USER_AGENT),
                    header("Referer", config::REFERER),
                ],
                4 * 1024 * 1024,
            )
            .await?;
        let programs = parse_epg(&body);
        if programs.is_empty() {
            return Err(NetError::Parse(format!("节目单为空: {livepid}/{date}")));
        }
        self.write_disk(livepid, &date, &programs).await;
        let programs = Arc::new(programs);
        self.mem
            .lock()
            .await
            .put(key, (Instant::now(), programs.clone()));
        Ok(programs)
    }

    pub async fn now_playing(&self, livepid: &str) -> Option<Program> {
        let programs = self.programs(livepid, "").await.ok()?;
        current_program(&programs, now_secs()).cloned()
    }
}
