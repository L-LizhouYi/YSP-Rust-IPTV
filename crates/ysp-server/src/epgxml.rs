//! `/epg.xml`：XMLTV 节目单（供 `channels.m3u` 的 url-tvg 使用）。

use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use tokio::sync::Semaphore;
use ysp_core::util::cn_offset;

use crate::app::AppState;

fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(text: &str) -> String {
    escape_text(text).replace('"', "&quot;")
}

fn xmltv_time(ts: i64) -> String {
    match Utc.timestamp_opt(ts, 0).single() {
        Some(dt) => dt
            .with_timezone(&cn_offset())
            .format("%Y%m%d%H%M%S +0800")
            .to_string(),
        None => String::new(),
    }
}

/// 带缓存 + 单飞的入口：5 分钟内复用同一份 XML。
pub async fn render_cached(state: &Arc<AppState>, date: &str) -> String {
    let _single = state.epg_render.lock().await;
    {
        let cache = state.epg_xml_cache.lock().await;
        if let Some((at, cached_date, xml)) = cache.as_ref() {
            if cached_date == date && at.elapsed() < Duration::from_secs(300) {
                return xml.clone();
            }
        }
    }
    let xml = render(state, date).await;
    *state.epg_xml_cache.lock().await =
        Some((std::time::Instant::now(), date.to_string(), xml.clone()));
    xml
}

/// 拉全部非 VIP 频道的节目单并渲染。单频道超时 6 秒，失败只跳过。
pub async fn render(state: &Arc<AppState>, date: &str) -> String {
    let channels = state.catalog.visible(false).await.unwrap_or_default();
    let sem = Arc::new(Semaphore::new(8));
    let mut handles = Vec::with_capacity(channels.len());

    for channel in channels.iter().cloned() {
        let sem = Arc::clone(&sem);
        let epg = Arc::clone(&state.epg);
        let date = date.to_string();
        handles.push(tokio::spawn(async move {
            let Ok(_permit) = sem.acquire_owned().await else {
                return (channel, None);
            };
            let programs = tokio::time::timeout(
                Duration::from_secs(4),
                epg.programs(&channel.livepid, &date),
            )
            .await
            .ok()
            .and_then(|result| result.ok());
            (channel, programs)
        }));
    }

    let mut xml = String::with_capacity(64 * 1024);
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<!DOCTYPE tv SYSTEM \"xmltv.dtd\">\n");
    xml.push_str("<tv generator-info-name=\"ysp-rs\">\n");

    let mut programmes = String::new();
    for handle in handles {
        let Ok((channel, programs)) = handle.await else {
            continue;
        };
        xml.push_str(&format!("  <channel id=\"{}\">", escape_attr(channel.id())));
        xml.push_str(&format!(
            "<display-name>{}</display-name>",
            escape_text(&channel.name)
        ));
        if !channel.logo.is_empty() {
            xml.push_str(&format!("<icon src=\"{}\"/>", escape_attr(&channel.logo)));
        }
        xml.push_str("</channel>\n");

        if let Some(programs) = programs {
            for program in programs.iter() {
                if program.end_ts <= program.start_ts {
                    continue;
                }
                programmes.push_str(&format!(
                    "  <programme start=\"{}\" stop=\"{}\" channel=\"{}\">\
<title lang=\"zh\">{}</title></programme>\n",
                    xmltv_time(program.start_ts),
                    xmltv_time(program.end_ts),
                    escape_attr(channel.id()),
                    escape_text(&program.name)
                ));
            }
        }
    }
    xml.push_str(&programmes);
    xml.push_str("</tv>\n");
    xml
}
