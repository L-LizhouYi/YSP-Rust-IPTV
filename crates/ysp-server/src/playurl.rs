//! playurl 缓存：每个频道一把锁，刷新单飞，互不阻塞。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Semaphore};
use ysp_core::models::{Channel, PlayInfo};
use ysp_net::{NetError, Player};

#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub info: Option<PlayInfo>,
    pub at: Option<Instant>,
    pub error: Option<String>,
}

pub struct PlayurlManager {
    player: Arc<Player>,
    ttl: Duration,
    error_ttl: Duration,
    slots: Mutex<HashMap<String, Arc<Mutex<Entry>>>>,
}

impl PlayurlManager {
    pub fn new(player: Arc<Player>, ttl: Duration, error_ttl: Duration) -> Self {
        Self {
            player,
            ttl,
            error_ttl,
            slots: Mutex::new(HashMap::new()),
        }
    }

    async fn slot(&self, livepid: &str) -> Arc<Mutex<Entry>> {
        let mut slots = self.slots.lock().await;
        slots
            .entry(livepid.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(Entry::default())))
            .clone()
    }

    fn fresh_info(entry: &Entry, ttl: Duration) -> Option<PlayInfo> {
        let info = entry.info.as_ref()?;
        if info.playurl.is_empty() {
            return None;
        }
        let at = entry.at?;
        if at.elapsed() < ttl {
            Some(info.clone())
        } else {
            None
        }
    }

    pub async fn peek(&self, livepid: &str) -> Option<Entry> {
        let slots = self.slots.lock().await;
        let slot = slots.get(livepid)?.clone();
        drop(slots);
        let entry = slot.lock().await;
        Some(entry.clone())
    }

    pub async fn snapshot(&self) -> HashMap<String, Entry> {
        let slots: Vec<(String, Arc<Mutex<Entry>>)> = {
            let map = self.slots.lock().await;
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };
        let mut out = HashMap::new();
        for (key, slot) in slots {
            let entry = slot.lock().await;
            out.insert(key, entry.clone());
        }
        out
    }

    /// 拿播放地址。同频道并发请求只会打一次上游；不同频道互不影响。
    pub async fn get(&self, channel: &Channel, force: bool) -> Result<PlayInfo, NetError> {
        let slot = self.slot(&channel.livepid).await;

        {
            let entry = slot.lock().await;
            if !force {
                if let Some(info) = Self::fresh_info(&entry, self.ttl) {
                    return Ok(info);
                }
                if let (Some(msg), Some(at)) = (&entry.error, entry.at) {
                    if at.elapsed() < self.error_ttl {
                        return Err(NetError::Upstream {
                            code: 0,
                            msg: msg.clone(),
                        });
                    }
                }
            }
        }

        // 单飞：拿锁之后重新检查，等待者直接复用结果。
        let mut entry = slot.lock().await;
        if !force {
            if let Some(info) = Self::fresh_info(&entry, self.ttl) {
                return Ok(info);
            }
        }

        let started = Instant::now();
        match self.player.pick_playable(channel).await {
            Ok(info) => {
                *entry = Entry {
                    info: Some(info.clone()),
                    at: Some(Instant::now()),
                    error: None,
                };
                tracing::info!(
                    "[{}] playurl {} ({:.2}s) cdn={}",
                    channel.name,
                    info.defn,
                    started.elapsed().as_secs_f64(),
                    info.cdn
                );
                Ok(info)
            }
            Err(err) => {
                *entry = Entry {
                    info: None,
                    at: Some(Instant::now()),
                    error: Some(err.to_string()),
                };
                Err(err)
            }
        }
    }

    /// 并发预热 / 批量刷新。`force=false` 时命中缓存直接跳过。
    pub async fn warm(self: &Arc<Self>, channels: Vec<Channel>, workers: usize, force: bool) {
        let sem = Arc::new(Semaphore::new(workers.max(1)));
        let mut handles = Vec::with_capacity(channels.len());
        for channel in channels {
            let sem = sem.clone();
            let this = Arc::clone(self);
            handles.push(tokio::spawn(async move {
                let Ok(_permit) = sem.acquire_owned().await else {
                    return;
                };
                if let Err(err) = this.get(&channel, force).await {
                    tracing::warn!("[{}] 不可播: {err}", channel.name);
                }
            }));
        }
        for handle in handles {
            let _ = handle.await;
        }
    }

    /// 后台定时刷新（默认 50 分钟）。刷新失败保留旧值，继续用旧 playurl。
    pub fn start_background(
        self: &Arc<Self>,
        channels: Arc<crate::app::CatalogService>,
        interval: Duration,
        workers: usize,
    ) {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let Ok(list) = channels.visible(false).await else {
                    continue;
                };
                let sem = Arc::new(Semaphore::new(workers.max(1)));
                for channel in list {
                    let sem = sem.clone();
                    let this = Arc::clone(&this);
                    tokio::spawn(async move {
                        let Ok(_permit) = sem.acquire_owned().await else {
                            return;
                        };
                        if let Err(err) = this.get(&channel, true).await {
                            tracing::warn!("[{}] 刷新失败: {err}", channel.name);
                        }
                    });
                }
            }
        });
    }
}
