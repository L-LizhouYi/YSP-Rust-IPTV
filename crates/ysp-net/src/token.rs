//! h5access 临时令牌客户端（按 (yspappid, guid) 缓存，5 分钟有效）。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::Mutex;
use ysp_core::{config, util::random_hex};

use crate::{
    error::NetError,
    http::{header, HttpClient},
};

pub struct TokenClient {
    http: HttpClient,
    cache: Mutex<HashMap<(String, String), (String, Instant)>>,
    fetch_lock: Mutex<()>,
}

impl TokenClient {
    pub fn new(http: HttpClient) -> Self {
        Self {
            http,
            cache: Mutex::new(HashMap::new()),
            fetch_lock: Mutex::new(()),
        }
    }

    pub fn build_url(guid: &str, yspappid: &str, ts_ms: u128, rnd: &str) -> String {
        format!(
            "{}?yspappid={}&guid={}&vappid={}&vsecret={}&raw=1&version={}&ts={}&rnd={}",
            config::H5ACCESS_URL,
            yspappid,
            guid,
            config::H5ACCESS_VAPPID,
            config::H5ACCESS_VSECRET,
            config::H5ACCESS_VERSION,
            ts_ms,
            rnd
        )
    }

    async fn fetch(&self, guid: &str, yspappid: &str) -> Result<(String, Instant), NetError> {
        let ts_ms = ysp_core::util::now_ms();
        let url = Self::build_url(guid, yspappid, ts_ms, &random_hex(16));
        let text = self
            .http
            .get_text(
                &url,
                &[
                    header("Content-Type", "application/json"),
                    header("Origin", config::ORIGIN),
                    header("Referer", config::REFERER),
                ],
                64 * 1024,
            )
            .await?;
        let data: Value = serde_json::from_str(&text)
            .map_err(|_| NetError::Parse(format!("h5access 返回非 JSON: {}", &text[..text.len().min(200)])))?;
        let ret = data.get("ret").and_then(Value::as_i64).unwrap_or(0);
        let inner = data.get("data").cloned().unwrap_or(Value::Null);
        let inner_code = inner.get("code").and_then(Value::as_i64).unwrap_or(0);
        if ret != 0 || inner_code != 0 {
            return Err(NetError::Upstream {
                code: ret,
                msg: format!("h5access: {}", &text[..text.len().min(200)]),
            });
        }
        let token = inner
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if token.is_empty() {
            return Err(NetError::Upstream {
                code: 0,
                msg: "h5access 未返回 token".into(),
            });
        }
        let expire_ms = inner.get("expire").and_then(Value::as_u64).unwrap_or(300_000);
        let expire_at = Instant::now() + Duration::from_millis(expire_ms);
        Ok((token, expire_at))
    }

    pub async fn token(&self, guid: &str, yspappid: &str) -> Result<String, NetError> {
        let key = (yspappid.to_string(), guid.to_string());
        if let Some(hit) = self.lookup(&key).await {
            return Ok(hit);
        }
        // 只允许一个请求去上游取 token，其余等待后命中缓存。
        let _guard = self.fetch_lock.lock().await;
        if let Some(hit) = self.lookup(&key).await {
            return Ok(hit);
        }
        let (token, expire_at) = self.fetch(guid, yspappid).await?;
        self.cache
            .lock()
            .await
            .insert(key, (token.clone(), expire_at));
        Ok(token)
    }

    async fn lookup(&self, key: &(String, String)) -> Option<String> {
        let cache = self.cache.lock().await;
        let (token, expire_at) = cache.get(key)?;
        // 留 10 秒余量
        if *expire_at > Instant::now() + Duration::from_secs(10) {
            Some(token.clone())
        } else {
            None
        }
    }

    pub async fn clear(&self) {
        self.cache.lock().await.clear();
    }
}
