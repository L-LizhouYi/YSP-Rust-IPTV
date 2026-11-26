//! HTTP 路由：页面、JSON API、HLS 代理。

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};
use ysp_core::{media::playlist, models::Channel, util::today_compact};
use ysp_net::NetError;

use crate::{app::AppState, epgxml, m3u, proxy::ProxyError, web};

static HLS_JS: &[u8] = include_bytes!("../assets/hls.min.js");

pub enum ApiError {
    Net(NetError),
    Proxy(ProxyError),
    NotFound(String),
}

impl From<NetError> for ApiError {
    fn from(err: NetError) -> Self {
        ApiError::Net(err)
    }
}

impl From<ProxyError> for ApiError {
    fn from(err: ProxyError) -> Self {
        ApiError::Proxy(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::Net(err) => (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_GATEWAY),
                err.to_string(),
            ),
            ApiError::Proxy(err) => (
                StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::BAD_GATEWAY),
                err.to_string(),
            ),
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message.clone()),
        };
        let body = json!({ "ok": false, "error": message }).to_string();
        let mut response = (status, body).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

fn json_response(value: Value) -> Response {
    let body = serde_json::to_string(&value).unwrap_or_else(|_| "{}".into());
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn html_response(body: String) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

fn xml_response(body: String) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=60"),
    );
    response
}

/// `/live/*.m3u8`：HLS 播放列表（adaptive/HLS 客户端按这个 MIME 处理）。
fn playlist_response(body: String) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.apple.mpegurl"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

/// `/channels.m3u`：**频道清单**，不是 HLS 媒体列表。
///
/// 注意别用 `application/vnd.apple.mpegurl`：VLC 会把它当成单条 HLS 流，
/// 交给 adaptive/avformat 解复用，导致整份列表不被展开。
/// `audio/x-mpegurl` 是 VLC 的 playlist stream_filter 认可的 M3U MIME。
fn m3u_response(body: String) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("audio/x-mpegurl; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

fn base_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(base) = &state.cfg.public_base {
        return base.clone();
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:8525");
    format!("http://{host}")
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/static/hls.min.js", get(hls_js))
        .route("/channels.m3u", get(channels_m3u))
        .route("/epg.xml", get(epg_xml))
        .route("/api/channels", get(api_channels))
        .route("/api/channels/*rest", get(api_channel))
        .route("/api/stats", get(api_stats))
        .route("/admin/refresh", post(admin_refresh))
        .route("/live/*rest", get(live))
        .route("/play/*rest", get(play_page))
        .fallback(not_found)
        .layer(axum::middleware::from_fn(cors))
        .with_state(state)
}

// ---------------------------------------------------------------- 中间件

async fn cors(request: Request<Body>, next: Next) -> Response {
    if request.method() == axum::http::Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, HEAD, OPTIONS, POST"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Range, Origin, Accept, Content-Type"),
        );
        return response;
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Content-Length, Content-Range"),
    );
    response
}

// ---------------------------------------------------------------- 页面

async fn index(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let channels = state.catalog.visible(true).await?;
    let status = state.playurls.snapshot().await;
    let rows: Vec<(Channel, Option<bool>)> = channels
        .into_iter()
        .map(|ch| {
            let playable = status.get(&ch.livepid).and_then(|entry| {
                if entry.info.is_some() {
                    Some(true)
                } else if entry.error.is_some() {
                    Some(false)
                } else {
                    None
                }
            });
            (ch, playable)
        })
        .collect();
    let base = base_url(&state, &headers);
    let stats = format!("{} 个频道", rows.len());
    Ok(html_response(web::catalog_page(&rows, &base, &stats)))
}

async fn play_page(
    State(state): State<Arc<AppState>>,
    Path(ident): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let channel = state
        .catalog
        .get(&ident)
        .await?
        .ok_or_else(|| ApiError::NotFound("未知频道".into()))?;
    let program = state.catalog.now_playing(&channel).await;
    let entry = state.playurls.peek(&channel.livepid).await;
    let (defn, formats, mode) = match entry.as_ref().and_then(|e| e.info.as_ref()) {
        Some(info) => (
            info.defn.clone(),
            info.formats.clone(),
            state.mode_for(info).as_str().to_string(),
        ),
        None => (String::new(), Vec::new(), "-".to_string()),
    };
    let playable = entry.as_ref().map(|e| e.info.is_some());
    let base = base_url(&state, &headers);
    Ok(html_response(web::player_page(
        &channel,
        &base,
        program.as_ref(),
        &defn,
        &formats,
        playable,
        &mode,
    )))
}

async fn healthz(State(state): State<Arc<AppState>>) -> Response {
    let body = format!(
        "ok uptime={}s cache={}/{}B\n",
        state.started.elapsed().as_secs(),
        state.proxies.budget().used(),
        state.proxies.budget().limit()
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
        .into_response()
}

async fn hls_js() -> Response {
    let mut response = (StatusCode::OK, HLS_JS).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/javascript; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=86400"),
    );
    response
}

async fn not_found() -> Response {
    ApiError::NotFound("not found".into()).into_response()
}

// ---------------------------------------------------------------- m3u / epg

async fn channels_m3u(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let include_vip = query
        .get("all")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let channels = state.catalog.visible(include_vip).await?;
    let base = base_url(&state, &headers);
    let body = m3u::render(&channels, &base);
    Ok(m3u_response(body))
}

async fn epg_xml(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let date = query
        .get("date")
        .cloned()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(today_compact);
    let xml = epgxml::render_cached(&state, &date).await;
    Ok(xml_response(xml))
}

// ---------------------------------------------------------------- JSON API

async fn api_channels(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let include_vip = query
        .get("all")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    if query
        .get("refresh")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
    {
        state.catalog.ensure_loaded(true).await?;
    }
    let channels = state.catalog.visible(include_vip).await?;
    let status = state.playurls.snapshot().await;
    let list: Vec<Value> = channels
        .iter()
        .map(|ch| {
            let entry = status.get(&ch.livepid);
            let playable = entry.and_then(|e| {
                if e.info.is_some() {
                    Some(true)
                } else if e.error.is_some() {
                    Some(false)
                } else {
                    None
                }
            });
            let (defn, formats) = entry
                .and_then(|e| e.info.as_ref())
                .map(|info| (info.defn.clone(), info.formats.clone()))
                .unwrap_or_default();
            ch.to_json(playable, &defn, &formats)
        })
        .collect();
    Ok(json_response(json!({
        "ok": true,
        "count": list.len(),
        "channels": list,
    })))
}

async fn api_channel(
    State(state): State<Arc<AppState>>,
    Path(rest): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let bits: Vec<&str> = rest.split('/').filter(|b| !b.is_empty()).collect();
    let Some(ident) = bits.first() else {
        return Err(ApiError::NotFound("not found".into()));
    };
    let channel = state
        .catalog
        .get(ident)
        .await?
        .ok_or_else(|| ApiError::NotFound("未知频道".into()))?;

    if bits.len() == 1 {
        let now = state.catalog.now_playing(&channel).await;
        let entry = state.playurls.peek(&channel.livepid).await;
        let playable = entry.as_ref().map(|e| e.info.is_some());
        let (defn, formats) = entry
            .as_ref()
            .and_then(|e| e.info.as_ref())
            .map(|info| (info.defn.clone(), info.formats.clone()))
            .unwrap_or_default();
        let base = base_url(&state, &headers);
        return Ok(json_response(json!({
            "ok": true,
            "channel": channel.to_json(playable, &defn, &formats),
            "now": now.map(|p| p.to_json()),
            "live": format!("{}/live/{}.m3u8", base, channel.id()),
        })));
    }

    match bits[1] {
        "epg" => {
            let date = query.get("date").cloned().unwrap_or_default();
            let programs = state.epg.programs(&channel.livepid, &date).await?;
            Ok(json_response(json!({
                "ok": true,
                "channel": channel.to_json(None, "", &[]),
                "programs": programs.iter().map(|p| p.to_json()).collect::<Vec<_>>(),
            })))
        }
        "playurl" => {
            let force = query
                .get("force")
                .map(|v| matches!(v.as_str(), "1" | "true"))
                .unwrap_or(false);
            let defn = query.get("defn").cloned().unwrap_or_default();
            let info = if defn.is_empty() {
                state.playurls.get(&channel, force).await?
            } else {
                state.player.live_info(&channel, &defn).await?
            };
            Ok(json_response(json!({
                "ok": true,
                "channel": channel.to_json(Some(true), &info.defn, &info.formats),
                "play": info.to_json(),
            })))
        }
        "refresh" => {
            state.playurls.get(&channel, true).await?;
            Ok(json_response(json!({
                "ok": true,
                "channel": channel.to_json(Some(true), "", &[]),
            })))
        }
        _ => Err(ApiError::NotFound("not found".into())),
    }
}

async fn admin_refresh(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    if let Some(ident) = query.get("channel").filter(|s| !s.is_empty()) {
        let channel = state
            .catalog
            .get(ident)
            .await?
            .ok_or_else(|| ApiError::NotFound("未知频道".into()))?;
        state.playurls.get(&channel, true).await?;
        return Ok(json_response(
            json!({"ok": true, "refreshed": [channel.name]}),
        ));
    }
    let channels = state.catalog.visible(false).await?;
    let total = channels.len();
    state.playurls.warm(channels, 4, true).await;
    let status = state.playurls.snapshot().await;
    let playable = status
        .values()
        .filter(|e| e.info.as_ref().map(|i| !i.playurl.is_empty()).unwrap_or(false))
        .count();
    Ok(json_response(json!({
        "ok": true,
        "total": total,
        "playable": playable,
    })))
}

async fn api_stats(State(state): State<Arc<AppState>>) -> Result<Response, ApiError> {
    let proxies = state.proxies.snapshot().await;
    Ok(json_response(json!({
        "ok": true,
        "uptime_secs": state.started.elapsed().as_secs(),
        "channels": state.catalog.count().await,
        "rss_mb": rss_mb(),
        "cache": {
            "used_bytes": state.proxies.budget().used(),
            "limit_bytes": state.proxies.budget().limit(),
        },
        "proxies": proxies,
    })))
}

#[cfg(target_os = "linux")]
fn rss_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = text.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096 / 1024 / 1024)
}

#[cfg(not(target_os = "linux"))]
fn rss_mb() -> Option<u64> {
    None
}

// ---------------------------------------------------------------- HLS

async fn live(
    State(state): State<Arc<AppState>>,
    Path(rest): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(ident) = rest.strip_suffix("/master.m3u8") {
        return master_playlist(state, ident, headers).await;
    }
    if let Some(ident) = rest.strip_suffix(".m3u8") {
        return media_playlist(state, ident).await;
    }
    if let Some((ident, segment)) = rest.split_once('/') {
        return segment_response(state, ident, segment, headers).await;
    }
    Err(ApiError::NotFound("not found".into()))
}

async fn master_playlist(
    state: Arc<AppState>,
    ident: &str,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let channel = state
        .catalog
        .get(ident)
        .await?
        .ok_or_else(|| ApiError::NotFound("未知频道".into()))?;
    let info = state.playurls.get(&channel, false).await?;
    let (bandwidth, resolution) = if info.defn == "fhd" {
        (4_000_000, "1920x1080")
    } else {
        (2_000_000, "1280x720")
    };
    let base = base_url(&state, &headers);
    let body = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n\
#EXT-X-STREAM-INF:BANDWIDTH={bandwidth},RESOLUTION={resolution},CODECS=\"{}\"\n\
{}/live/{}.m3u8\n",
        playlist::CODECS_HINT,
        base,
        channel.id()
    );
    Ok(playlist_response(body))
}

async fn media_playlist(state: Arc<AppState>, ident: &str) -> Result<Response, ApiError> {
    let channel = state
        .catalog
        .get(ident)
        .await?
        .ok_or_else(|| ApiError::NotFound("未知频道".into()))?;
    let info = state.playurls.get(&channel, false).await?;
    let mode = state.mode_for(&info);
    let proxy = state
        .proxies
        .get_or_create(&channel, &info.playurl, mode, &info.defn)
        .await;
    let body = proxy.rewritten().await?;
    Ok(playlist_response(body))
}

async fn segment_response(
    state: Arc<AppState>,
    ident: &str,
    name: &str,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let channel = state
        .catalog
        .get(ident)
        .await?
        .ok_or_else(|| ApiError::NotFound("未知频道".into()))?;
    let info = state.playurls.get(&channel, false).await?;
    let mode = state.mode_for(&info);
    let proxy = state
        .proxies
        .get_or_create(&channel, &info.playurl, mode, &info.defn)
        .await;
    let data = proxy.segment(name).await?;
    let total = data.len();

    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_range(value, total));

    if let Some((start, end)) = range {
        let chunk = data.slice(start..=end);
        let mut response = (StatusCode::PARTIAL_CONTENT, chunk).into_response();
        let response_headers = response.headers_mut();
        response_headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("video/mp2t"),
        );
        response_headers.insert(
            header::ACCEPT_RANGES,
            HeaderValue::from_static("bytes"),
        );
        response_headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
                .unwrap_or(HeaderValue::from_static("")),
        );
        return Ok(response);
    }

    let mut response = (StatusCode::OK, data).into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("video/mp2t"),
    );
    response_headers.insert(
        header::ACCEPT_RANGES,
        HeaderValue::from_static("bytes"),
    );
    Ok(response)
}

/// 解析 `bytes=start-end`（单段），返回闭区间。
pub fn parse_range(header_value: &str, total: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let rest = header_value.trim().strip_prefix("bytes=")?;
    if rest.contains(',') {
        return None;
    }
    let (start_text, end_text) = rest.split_once('-')?;
    if start_text.is_empty() && end_text.is_empty() {
        return None;
    }
    if start_text.is_empty() {
        let length: usize = end_text.trim().parse().ok()?;
        let start = total.saturating_sub(length);
        return Some((start, total - 1));
    }
    let start: usize = start_text.trim().parse().ok()?;
    if start >= total {
        return None;
    }
    let end = if end_text.is_empty() {
        total - 1
    } else {
        end_text.trim().parse::<usize>().ok()?.min(total - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-", 1000), Some((100, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=900-2000", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 1000), None);
    }
}
