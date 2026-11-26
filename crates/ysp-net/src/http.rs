//! 统一 HTTP 客户端：UA / Referer / 超时 / 重试 / 响应大小上限。

use std::time::Duration;

use bytes::{Bytes, BytesMut};
use reqwest::header::CONTENT_TYPE;
use reqwest::Method;
use ysp_core::config;

use crate::error::NetError;

pub type Header = (String, String);

/// 便捷构造 header。
pub fn header(name: &str, value: impl Into<String>) -> Header {
    (name.to_string(), value.into())
}

#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
}

impl HttpClient {
    pub fn new() -> Result<Self, NetError> {
        let client = reqwest::Client::builder()
            .user_agent(config::USER_AGENT)
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .build()
            .map_err(|e| NetError::Transport(format!("初始化 HTTP 客户端失败: {e}")))?;
        Ok(Self { client })
    }

    fn base_headers() -> Vec<Header> {
        vec![
            header("Accept", "*/*"),
            header("Accept-Language", "zh-CN"),
        ]
    }

    pub async fn get(
        &self,
        url: &str,
        extra: &[Header],
        limit: usize,
    ) -> Result<Bytes, NetError> {
        self.request(Method::GET, url, None, None, extra, limit).await
    }

    pub async fn get_text(
        &self,
        url: &str,
        extra: &[Header],
        limit: usize,
    ) -> Result<String, NetError> {
        let bytes = self.get(url, extra, limit).await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub async fn post_form(
        &self,
        url: &str,
        body: &str,
        extra: &[Header],
        limit: usize,
    ) -> Result<String, NetError> {
        let bytes = self
            .request(
                Method::POST,
                url,
                Some("application/x-www-form-urlencoded;charset=UTF-8"),
                Some(body.as_bytes()),
                extra,
                limit,
            )
            .await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub async fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        extra: &[Header],
        limit: usize,
    ) -> Result<String, NetError> {
        let text = serde_json::to_vec(body)
            .map_err(|e| NetError::Parse(format!("序列化请求体失败: {e}")))?;
        let bytes = self
            .request(
                Method::POST,
                url,
                Some("application/json;charset=UTF-8"),
                Some(&text),
                extra,
                limit,
            )
            .await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn request(
        &self,
        method: Method,
        url: &str,
        content_type: Option<&str>,
        body: Option<&[u8]>,
        extra: &[Header],
        limit: usize,
    ) -> Result<Bytes, NetError> {
        let mut last: Option<NetError> = None;
        for attempt in 0..3u64 {
            match self
                .send_once(method.clone(), url, content_type, body, extra, limit)
                .await
            {
                Ok(bytes) => return Ok(bytes),
                Err(err) if err.retryable() && attempt < 2 => {
                    tracing::debug!("重试 {url}: {err}");
                    last = Some(err);
                    tokio::time::sleep(Duration::from_millis(400 * (attempt + 1))).await;
                }
                Err(err) => return Err(err),
            }
        }
        Err(last.unwrap_or_else(|| NetError::Transport(format!("{method} {url} 失败"))))
    }

    async fn send_once(
        &self,
        method: Method,
        url: &str,
        content_type: Option<&str>,
        body: Option<&[u8]>,
        extra: &[Header],
        limit: usize,
    ) -> Result<Bytes, NetError> {
        let mut req = self.client.request(method.clone(), url);
        for (k, v) in Self::base_headers() {
            req = req.header(k, v);
        }
        for (k, v) in extra {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(ct) = content_type {
            req = req.header(CONTENT_TYPE, ct);
        }
        if let Some(data) = body {
            req = req.body(data.to_vec());
        }
        let resp = req
            .send()
            .await
            .map_err(|e| NetError::Transport(format!("{method} {url}: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let snippet: String = text.chars().take(200).collect();
            return Err(NetError::HttpStatus {
                status: status.as_u16(),
                url: url.to_string(),
                body: snippet,
            });
        }
        read_limited(resp, limit, url).await
    }
}

async fn read_limited(
    mut resp: reqwest::Response,
    limit: usize,
    url: &str,
) -> Result<Bytes, NetError> {
    if let Some(len) = resp.content_length() {
        if len as usize > limit {
            return Err(NetError::TooLarge(format!("{url}: Content-Length={len}")));
        }
    }
    let mut buf = BytesMut::with_capacity(
        resp.content_length().unwrap_or(32 * 1024).min(limit as u64) as usize,
    );
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| NetError::Transport(format!("{url}: 读取响应失败: {e}")))?
    {
        if buf.len() + chunk.len() > limit {
            return Err(NetError::TooLarge(format!("{url}: 超过 {limit} 字节")));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf.freeze())
}
