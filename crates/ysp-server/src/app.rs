//! 应用装配：目录服务 + 全局状态（唯一 new 具体实现的地方）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, RwLock};
use ysp_core::{
    config,
    media::DecryptMode,
    models::{Channel, PlayInfo, Program},
};
use ysp_net::{
    catalog::CatalogClient,
    epg::EpgClient,
    http::HttpClient,
    player::Player,
    signer::Signer,
    NetError,
};

use crate::{playurl::PlayurlManager, proxy::ProxyRegistry, settings::Settings};

/// 目录 + 「是否可播」状态。5 分钟内复用内存快照。
pub struct CatalogService {
    client: CatalogClient,
    epg: Arc<EpgClient>,
    state: RwLock<CatalogState>,
    load_lock: Mutex<()>,
}

struct CatalogState {
    channels: Vec<Channel>,
    index: HashMap<String, usize>,
    loaded_at: Option<Instant>,
}

impl CatalogService {
    pub fn new(client: CatalogClient, epg: Arc<EpgClient>) -> Self {
        Self {
            client,
            epg,
            state: RwLock::new(CatalogState {
                channels: Vec::new(),
                index: HashMap::new(),
                loaded_at: None,
            }),
            load_lock: Mutex::new(()),
        }
    }

    pub async fn ensure_loaded(&self, force: bool) -> Result<(), NetError> {
        {
            let state = self.state.read().await;
            let fresh = state
                .loaded_at
                .map(|at| at.elapsed() < Duration::from_secs(300))
                .unwrap_or(false);
            if !force && fresh && !state.channels.is_empty() {
                return Ok(());
            }
        }

        let _single = self.load_lock.lock().await;
        {
            let state = self.state.read().await;
            let fresh = state
                .loaded_at
                .map(|at| at.elapsed() < Duration::from_secs(300))
                .unwrap_or(false);
            if !force && fresh && !state.channels.is_empty() {
                return Ok(());
            }
        }

        let channels = self.client.load(force).await?;
        let mut index = HashMap::with_capacity(channels.len() * 3);
        for (pos, ch) in channels.iter().enumerate() {
            index.insert(ch.livepid.clone(), pos);
            index.insert(ch.id().to_string(), pos);
            index.insert(ch.name.to_lowercase(), pos);
        }
        tracing::info!("目录加载完成: {} 个频道", channels.len());
        let mut state = self.state.write().await;
        state.channels = channels;
        state.index = index;
        state.loaded_at = Some(Instant::now());
        Ok(())
    }

    pub async fn all(&self) -> Result<Vec<Channel>, NetError> {
        self.ensure_loaded(false).await?;
        Ok(self.state.read().await.channels.clone())
    }

    pub async fn visible(&self, include_vip: bool) -> Result<Vec<Channel>, NetError> {
        let channels = self.all().await?;
        Ok(if include_vip {
            channels
        } else {
            channels.into_iter().filter(|c| c.tier != "vip").collect()
        })
    }

    pub async fn get(&self, ident: &str) -> Result<Option<Channel>, NetError> {
        self.ensure_loaded(false).await?;
        let ident = ident.trim();
        let lowered = ident.to_lowercase();
        let state = self.state.read().await;
        if let Some(pos) = state.index.get(ident).or_else(|| state.index.get(&lowered)) {
            if let Some(ch) = state.channels.get(*pos) {
                return Ok(Some(ch.clone()));
            }
        }
        Ok(CatalogClient::find(&state.channels, ident).cloned())
    }

    pub async fn count(&self) -> usize {
        self.state.read().await.channels.len()
    }

    pub async fn now_playing(&self, ch: &Channel) -> Option<Program> {
        self.epg.now_playing(&ch.livepid).await
    }
}

pub struct AppState {
    pub cfg: Arc<Settings>,
    pub http: HttpClient,
    pub player: Arc<Player>,
    pub playurls: Arc<PlayurlManager>,
    pub catalog: Arc<CatalogService>,
    pub proxies: Arc<ProxyRegistry>,
    pub epg: Arc<EpgClient>,
    /// (生成时间, date, xml)
    pub epg_xml_cache: Mutex<Option<(Instant, String, String)>>,
    /// XMLTV 渲染单飞，避免首次并发请求重复拉 54 个频道
    pub epg_render: Mutex<()>,
    pub started: Instant,
}

impl AppState {
    pub fn build(cfg: Arc<Settings>) -> Result<Arc<Self>, NetError> {
        let http = HttpClient::new()?;
        let signer = Signer::new(http.clone());
        let player = Arc::new(Player::new(http.clone(), signer, cfg.guid.clone()));
        let playurls = Arc::new(PlayurlManager::new(
            Arc::clone(&player),
            Duration::from_secs(config::PLAYURL_TTL_SECS),
            Duration::from_secs(config::PLAYURL_ERROR_TTL_SECS),
        ));
        let epg = Arc::new(EpgClient::new(
            http.clone(),
            cfg.cache_dir.clone(),
            Duration::from_secs(config::EPG_TTL_SECS),
        ));
        let catalog = Arc::new(CatalogService::new(
            CatalogClient::new(
                http.clone(),
                cfg.cache_dir.clone(),
                Duration::from_secs(config::CATALOG_TTL_SECS),
            ),
            Arc::clone(&epg),
        ));
        let proxies = Arc::new(ProxyRegistry::new(
            http.clone(),
            cfg.segment_mem_limit,
            cfg.channel_mem_limit,
            cfg.playlist_ttl,
        ));

        Ok(Arc::new(Self {
            cfg,
            http,
            player,
            playurls,
            catalog,
            proxies,
            epg,
            epg_xml_cache: Mutex::new(None),
            epg_render: Mutex::new(()),
            started: Instant::now(),
        }))
    }

    /// 解密模式：优先环境变量覆盖，否则按官方 `encrypt` 字段。
    pub fn mode_for(&self, info: &PlayInfo) -> DecryptMode {
        self.cfg
            .decrypt_override
            .unwrap_or_else(|| DecryptMode::from_encrypt(info.encrypt))
    }
}
