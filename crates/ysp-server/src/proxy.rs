//! 单频道 HLS 代理。
//!
//! 并发/内存设计：
//! * 播放列表：`up` 只读锁快速命中，真正刷新时用 `pl_fetch` 单飞，不会排队阻塞分片路径；
//! * 分片：同频道同名分片用 `OnceCell` 单飞 —— N 个客户端只下载+解密一次，共享 `Bytes`（零拷贝）；
//! * 解密：CPU 密集，全部丢进 `spawn_blocking`，用信号量限制并行度，不阻塞 tokio 线程；
//! * 缓存：LRU + 全局字节预算 + 单频道预算，超预算先淘汰最旧；空闲频道由 reaper 回收。

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use lru::LruCache;
use serde::Serialize;
use tokio::sync::{Mutex, OnceCell, RwLock, Semaphore};
use ysp_core::{
    config,
    media::{self, playlist, DecryptMode},
    models::Channel,
};
use ysp_net::{
    http::{header, Header},
    HttpClient, NetError,
};

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("未知分片: {0}")]
    UnknownSegment(String),

    #[error("播放列表获取失败: {0}")]
    Playlist(String),

    #[error("分片获取失败: {0}")]
    Upstream(String),
}

impl From<NetError> for ProxyError {
    fn from(err: NetError) -> Self {
        ProxyError::Upstream(err.to_string())
    }
}

impl ProxyError {
    pub fn http_status(&self) -> u16 {
        match self {
            ProxyError::UnknownSegment(_) => 404,
            ProxyError::Playlist(_) | ProxyError::Upstream(_) => 502,
        }
    }
}

/// 全站分片缓存预算。
pub struct MemBudget {
    limit: usize,
    used: AtomicUsize,
}

impl MemBudget {
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: AtomicUsize::new(0),
        })
    }

    fn try_reserve(&self, bytes: usize) -> bool {
        let mut current = self.used.load(Ordering::Relaxed);
        loop {
            if current + bytes > self.limit {
                return false;
            }
            match self.used.compare_exchange_weak(
                current,
                current + bytes,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(value) => current = value,
            }
        }
    }

    fn release(&self, bytes: usize) {
        // 用 saturating_sub 防止重复释放把计数打回一个巨大的值。
        let _ = self
            .used
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(bytes))
            });
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
}

struct UpstreamState {
    url: String,
    mode: DecryptMode,
    defn: String,
    pl_text: String,
    pl_media_url: String,
    pl_at: Instant,
}

struct SegStore {
    urls: HashMap<String, (String, Instant)>,
    cache: LruCache<String, Bytes>,
    cached_bytes: usize,
    inflight: HashMap<String, Arc<OnceCell<Result<Bytes, String>>>>,
    last_access: Instant,
    hits: u64,
    misses: u64,
}

impl SegStore {
    fn new() -> Self {
        Self {
            urls: HashMap::new(),
            cache: LruCache::new(NonZeroUsize::new(256).unwrap()),
            cached_bytes: 0,
            inflight: HashMap::new(),
            last_access: Instant::now(),
            hits: 0,
            misses: 0,
        }
    }

    fn register(&mut self, abs: &str, mut name: String) -> String {
        if let Some((old, _)) = self.urls.get(&name) {
            if old != abs {
                let base = name.trim_end_matches(".ts").to_string();
                name = format!("{}-{:04x}.ts", base, fnv1a(abs) & 0xFFFF);
            }
        }
        self.urls.insert(name.clone(), (abs.to_string(), Instant::now()));
        if self.urls.len() > 512 {
            let now = Instant::now();
            self.urls
                .retain(|_, (_, at)| now.duration_since(*at) < Duration::from_secs(180));
            if self.urls.len() > 512 {
                let mut ages: Vec<(String, Instant)> = self
                    .urls
                    .iter()
                    .map(|(k, (_, at))| (k.clone(), *at))
                    .collect();
                ages.sort_by_key(|(_, at)| *at);
                for (key, _) in ages.into_iter().take(self.urls.len() / 2) {
                    self.urls.remove(&key);
                }
            }
        }
        name
    }
}

fn fnv1a(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[derive(Debug, Clone, Serialize)]
pub struct SegmentStats {
    pub id: String,
    pub upstream: String,
    pub defn: String,
    pub mode: String,
    pub cached_segments: usize,
    pub cached_bytes: usize,
    pub known_urls: usize,
    pub hits: u64,
    pub misses: u64,
}

pub struct ChannelProxy {
    pub id: String,
    http: HttpClient,
    budget: Arc<MemBudget>,
    /// `SegStore.cached_bytes` 的镜像，供 Drop 时归还预算（Drop 里不能 await）
    cached_bytes: AtomicUsize,
    channel_limit: usize,
    decrypt_permits: Arc<Semaphore>,
    prefetch_permits: Arc<Semaphore>,
    playlist_ttl: Duration,
    up: RwLock<UpstreamState>,
    pl_fetch: Mutex<()>,
    segs: Mutex<SegStore>,
}

impl ChannelProxy {
    pub fn new(
        ch: &Channel,
        playurl: &str,
        mode: DecryptMode,
        defn: &str,
        http: HttpClient,
        budget: Arc<MemBudget>,
        channel_limit: usize,
        decrypt_permits: Arc<Semaphore>,
        prefetch_permits: Arc<Semaphore>,
        playlist_ttl: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: ch.id().to_string(),
            http,
            budget,
            cached_bytes: AtomicUsize::new(0),
            channel_limit,
            decrypt_permits,
            prefetch_permits,
            playlist_ttl,
            up: RwLock::new(UpstreamState {
                url: playurl.to_string(),
                mode,
                defn: defn.to_string(),
                pl_text: String::new(),
                pl_media_url: String::new(),
                pl_at: Instant::now() - Duration::from_secs(3600),
            }),
            pl_fetch: Mutex::new(()),
            segs: Mutex::new(SegStore::new()),
        })
    }

    fn stream_headers(&self) -> Vec<Header> {
        vec![
            header("User-Agent", config::USER_AGENT),
            header("Referer", config::REFERER),
        ]
    }

    /// playurl 刷新后同步上游；地址变化时清空播放列表与分片缓存。
    pub async fn ensure_upstream(&self, url: &str, mode: DecryptMode, defn: &str) {
        {
            let up = self.up.read().await;
            if up.url == url && up.mode == mode {
                return;
            }
        }
        {
            let mut up = self.up.write().await;
            if up.url == url && up.mode == mode {
                return;
            }
            tracing::info!(
                "[{}] 上游切换 ({} -> {}, {} -> {})",
                self.id,
                up.defn,
                defn,
                up.mode.as_str(),
                mode.as_str()
            );
            up.url = url.to_string();
            up.mode = mode;
            up.defn = defn.to_string();
            up.pl_text.clear();
            up.pl_media_url.clear();
            up.pl_at = Instant::now() - Duration::from_secs(3600);
        }
        self.trim().await;
    }

    async fn fetch_playlist(&self, upstream: &str) -> Result<(String, String), ProxyError> {
        let mut text = self
            .http
            .get_text(upstream, &self.stream_headers(), config::PLAYLIST_MAX_BYTES)
            .await
            .map_err(|e| ProxyError::Playlist(e.to_string()))?;
        let mut media_url = upstream.to_string();
        if playlist::is_master(&text) {
            media_url = playlist::pick_variant(&text, upstream, 0);
            text = self
                .http
                .get_text(&media_url, &self.stream_headers(), config::PLAYLIST_MAX_BYTES)
                .await
                .map_err(|e| ProxyError::Playlist(e.to_string()))?;
        }
        Ok((text, media_url))
    }

    pub async fn playlist(&self) -> Result<(String, String), ProxyError> {
        {
            let up = self.up.read().await;
            if !up.pl_text.is_empty() && up.pl_at.elapsed() < self.playlist_ttl {
                return Ok((up.pl_text.clone(), up.pl_media_url.clone()));
            }
        }

        let _single = self.pl_fetch.lock().await;
        {
            let up = self.up.read().await;
            if !up.pl_text.is_empty() && up.pl_at.elapsed() < self.playlist_ttl {
                return Ok((up.pl_text.clone(), up.pl_media_url.clone()));
            }
        }

        let upstream = { self.up.read().await.url.clone() };
        match self.fetch_playlist(&upstream).await {
            Ok((text, media_url)) => {
                let mut up = self.up.write().await;
                up.pl_text = text.clone();
                up.pl_media_url = media_url.clone();
                up.pl_at = Instant::now();
                Ok((text, media_url))
            }
            Err(err) => {
                // 上游抖一下时继续用 30 秒内的旧列表，避免客户端集体卡住。
                let up = self.up.read().await;
                if !up.pl_text.is_empty() && up.pl_at.elapsed() < Duration::from_secs(30) {
                    tracing::warn!("[{}] 播放列表刷新失败，沿用旧列表: {err}", self.id);
                    Ok((up.pl_text.clone(), up.pl_media_url.clone()))
                } else {
                    Err(err)
                }
            }
        }
    }

    /// 拉取播放列表并把分片 URL 登记到本地表。
    async fn refresh_and_register(&self) -> Result<(String, Option<String>), ProxyError> {
        let (text, media_url) = self.playlist().await?;
        let prefix = format!("/live/{}/", self.id);
        let mut segs = self.segs.lock().await;
        segs.last_access = Instant::now();
        Ok(playlist::rewrite(&text, &media_url, &prefix, |abs, name| {
            segs.register(abs, name)
        }))
    }

    /// 拉取并按需重写播放列表（分片地址换成本地路径）。
    pub async fn rewritten(self: &Arc<Self>) -> Result<String, ProxyError> {
        let (out, last) = self.refresh_and_register().await?;
        if let Some(name) = last {
            self.spawn_prefetch(name);
        }
        Ok(out)
    }

    fn spawn_prefetch(self: &Arc<Self>, name: String) {
        let this = Arc::clone(self);
        let permits = Arc::clone(&self.prefetch_permits);
        tokio::spawn(async move {
            let Ok(_permit) = permits.acquire_owned().await else {
                return;
            };
            if let Err(err) = this.segment(&name).await {
                tracing::debug!("[{}] 预取 {name} 失败: {err}", this.id);
            }
        });
    }

    async fn lookup_url(&self, name: &str) -> Option<String> {
        self.segs
            .lock()
            .await
            .urls
            .get(name)
            .map(|(url, _)| url.clone())
    }

    async fn fetch_and_decrypt(&self, url: &str) -> Result<Bytes, String> {
        let raw = self
            .http
            .get(url, &self.stream_headers(), config::SEGMENT_MAX_BYTES)
            .await
            .map_err(|e| e.to_string())?;
        let mode = self.up.read().await.mode;
        if mode == DecryptMode::None {
            return Ok(raw);
        }
        let raw_len = raw.len();
        let permit = self
            .decrypt_permits
            .acquire()
            .await
            .map_err(|e| e.to_string())?;
        let joined = tokio::task::spawn_blocking(move || media::decrypt(mode, &raw))
            .await
            .map_err(|e| format!("解密任务失败: {e}"))?;
        drop(permit);
        let (data, stats) = joined;
        tracing::debug!(
            "[{}] {} {} -> {} bytes nals={} cells={} pes={}",
            self.id,
            mode.as_str(),
            raw_len,
            data.len(),
            stats.nals,
            stats.cells,
            stats.pes
        );
        Ok(Bytes::from(data))
    }

    pub async fn segment(&self, name: &str) -> Result<Bytes, ProxyError> {
        {
            let mut segs = self.segs.lock().await;
            segs.last_access = Instant::now();
            if let Some(bytes) = segs.cache.get(name).cloned() {
                segs.hits += 1;
                return Ok(bytes);
            }
            segs.misses += 1;
        }

        let url = match self.lookup_url(name).await {
            Some(url) => url,
            None => {
                // 客户端可能拿着上一版播放列表（代理刚重启/上游刚切换）：
                // 强制刷新一次并重新登记，再找不到才算真 404。
                tracing::info!("[{}] 未知分片 {name}，强制刷新播放列表", self.id);
                {
                    let mut up = self.up.write().await;
                    up.pl_at = Instant::now() - Duration::from_secs(3600);
                }
                let _ = self.refresh_and_register().await;
                match self.lookup_url(name).await {
                    Some(url) => url,
                    None => return Err(ProxyError::UnknownSegment(name.to_string())),
                }
            }
        };

        // 同分片单飞：第一个请求下载+解密，其余等待同一个结果。
        let cell = {
            let mut segs = self.segs.lock().await;
            segs.inflight
                .entry(name.to_string())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };

        let fetched = cell.get_or_init(|| self.fetch_and_decrypt(&url)).await.clone();

        {
            let mut segs = self.segs.lock().await;
            if let Some(existing) = segs.inflight.get(name) {
                if Arc::ptr_eq(existing, &cell) {
                    segs.inflight.remove(name);
                }
            }
            match &fetched {
                Ok(bytes) => self.insert_cache(&mut segs, name, bytes.clone()),
                Err(_) => {}
            }
        }

        fetched.map_err(ProxyError::Upstream)
    }

    fn insert_cache(&self, segs: &mut SegStore, name: &str, data: Bytes) {
        let size = data.len();
        if size == 0 || size > self.channel_limit {
            return;
        }
        if let Some(old) = segs.cache.pop(name) {
            segs.cached_bytes = segs.cached_bytes.saturating_sub(old.len());
            self.budget.release(old.len());
        }
        while segs.cached_bytes + size > self.channel_limit {
            match segs.cache.pop_lru() {
                Some((_, old)) => {
                    segs.cached_bytes = segs.cached_bytes.saturating_sub(old.len());
                    self.budget.release(old.len());
                }
                None => break,
            }
        }
        let mut reserved = false;
        loop {
            if self.budget.try_reserve(size) {
                reserved = true;
                break;
            }
            match segs.cache.pop_lru() {
                Some((_, old)) => {
                    segs.cached_bytes = segs.cached_bytes.saturating_sub(old.len());
                    self.budget.release(old.len());
                }
                None => break,
            }
        }
        if reserved {
            segs.cached_bytes += size;
            segs.cache.put(name.to_string(), data);
        }
        self.cached_bytes
            .store(segs.cached_bytes, Ordering::Relaxed);
    }

    /// 释放本频道的缓存（地址切换 / 空闲回收）。
    pub async fn trim(&self) {
        let mut freed = 0usize;
        {
            let mut segs = self.segs.lock().await;
            while let Some((_, bytes)) = segs.cache.pop_lru() {
                freed += bytes.len();
            }
            segs.cached_bytes = 0;
            segs.urls.clear();
            segs.inflight.clear();
        }
        self.cached_bytes.store(0, Ordering::Relaxed);
        if freed > 0 {
            self.budget.release(freed);
        }
    }

    pub async fn stats(&self) -> SegmentStats {
        let up = self.up.read().await;
        let segs = self.segs.lock().await;
        SegmentStats {
            id: self.id.clone(),
            upstream: up.url.clone(),
            defn: up.defn.clone(),
            mode: up.mode.as_str().to_string(),
            cached_segments: segs.cache.len(),
            cached_bytes: segs.cached_bytes,
            known_urls: segs.urls.len(),
            hits: segs.hits,
            misses: segs.misses,
        }
    }

    async fn idle_for(&self) -> Duration {
        self.segs.lock().await.last_access.elapsed()
    }
}

impl Drop for ChannelProxy {
    fn drop(&mut self) {
        // 代理被移出注册表时，把仍占用的预算还回去，避免全局计数泄漏。
        let bytes = self.cached_bytes.load(Ordering::Relaxed);
        if bytes > 0 {
            self.budget.release(bytes);
        }
    }
}

/// 频道 -> 代理实例。强引用保存（分片缓存要跨请求复用），
/// 由 reaper 按空闲时间裁剪缓存 / 移除实例。
pub struct ProxyRegistry {
    inner: Mutex<HashMap<String, Arc<ChannelProxy>>>,
    budget: Arc<MemBudget>,
    http: HttpClient,
    decrypt_permits: Arc<Semaphore>,
    prefetch_permits: Arc<Semaphore>,
    channel_limit: usize,
    playlist_ttl: Duration,
}

impl ProxyRegistry {
    pub fn new(
        http: HttpClient,
        segment_mem_limit: usize,
        channel_mem_limit: usize,
        playlist_ttl: Duration,
    ) -> Self {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        Self {
            inner: Mutex::new(HashMap::new()),
            budget: MemBudget::new(segment_mem_limit),
            http,
            decrypt_permits: Arc::new(Semaphore::new(cores.clamp(2, 8))),
            prefetch_permits: Arc::new(Semaphore::new(4)),
            channel_limit: channel_mem_limit,
            playlist_ttl,
        }
    }

    pub fn budget(&self) -> &Arc<MemBudget> {
        &self.budget
    }

    pub async fn get_or_create(
        &self,
        ch: &Channel,
        playurl: &str,
        mode: DecryptMode,
        defn: &str,
    ) -> Arc<ChannelProxy> {
        let mut map = self.inner.lock().await;
        if let Some(proxy) = map.get(ch.id()) {
            let proxy = Arc::clone(proxy);
            drop(map);
            proxy.ensure_upstream(playurl, mode, defn).await;
            return proxy;
        }
        let proxy = ChannelProxy::new(
            ch,
            playurl,
            mode,
            defn,
            self.http.clone(),
            Arc::clone(&self.budget),
            self.channel_limit,
            Arc::clone(&self.decrypt_permits),
            Arc::clone(&self.prefetch_permits),
            self.playlist_ttl,
        );
        map.insert(ch.id().to_string(), Arc::clone(&proxy));
        drop(map);
        proxy
    }

    /// 空闲 2 分钟裁剪缓存；空闲 10 分钟直接移除代理实例。
    pub async fn reap(&self, idle: Duration) {
        let proxies: Vec<(String, Arc<ChannelProxy>)> = {
            let map = self.inner.lock().await;
            map.iter().map(|(k, v)| (k.clone(), Arc::clone(v))).collect()
        };
        let mut remove = Vec::new();
        for (id, proxy) in proxies {
            let idle_for = proxy.idle_for().await;
            if idle_for > Duration::from_secs(600) {
                remove.push(id);
            } else if idle_for > idle {
                proxy.trim().await;
            }
        }
        if !remove.is_empty() {
            let mut map = self.inner.lock().await;
            for id in remove {
                map.remove(&id);
            }
        }
        tracing::debug!(
            "缓存回收: {} / {} 字节",
            self.budget.used(),
            self.budget.limit()
        );
    }

    pub async fn snapshot(&self) -> Vec<SegmentStats> {
        let proxies: Vec<Arc<ChannelProxy>> = {
            let map = self.inner.lock().await;
            map.values().map(Arc::clone).collect()
        };
        let mut out = Vec::with_capacity(proxies.len());
        for proxy in proxies {
            out.push(proxy.stats().await);
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn start_reaper(self: &Arc<Self>, interval: Duration) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                this.reap(Duration::from_secs(120)).await;
            }
        });
    }
}
