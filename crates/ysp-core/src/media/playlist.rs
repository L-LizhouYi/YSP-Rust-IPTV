//! HLS 播放列表工具（纯函数）。

use url::Url;

fn join_url(base: &str, rel: &str) -> String {
    match Url::parse(base).and_then(|b| b.join(rel)) {
        Ok(u) => u.to_string(),
        Err(_) => rel.to_string(),
    }
}

pub fn is_master(text: &str) -> bool {
    text.contains("#EXT-X-STREAM-INF")
}

/// 从 master playlist 选变体，默认最高码率；`max_bandwidth > 0` 时取不超过该码率的最高档。
pub fn pick_variant(master: &str, base_url: &str, max_bandwidth: u64) -> String {
    let lines: Vec<&str> = master.lines().collect();
    let mut variants: Vec<(u64, String)> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let stripped = line.trim();
        if !stripped.starts_with("#EXT-X-STREAM-INF") {
            continue;
        }
        let bandwidth = stripped
            .split("BANDWIDTH=")
            .nth(1)
            .map(|rest| {
                rest.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
            })
            .and_then(|digits| digits.parse::<u64>().ok())
            .unwrap_or(0);
        for nxt in lines.iter().skip(idx + 1) {
            let nxt = nxt.trim();
            if !nxt.is_empty() && !nxt.starts_with('#') {
                variants.push((bandwidth, join_url(base_url, nxt)));
                break;
            }
        }
    }
    if variants.is_empty() {
        return base_url.to_string();
    }
    if max_bandwidth > 0 {
        let eligible: Vec<&(u64, String)> = variants
            .iter()
            .filter(|(bw, _)| *bw <= max_bandwidth)
            .collect();
        if let Some((_, url)) = eligible.iter().max_by_key(|(bw, _)| *bw) {
            return (*url).clone();
        }
    }
    variants
        .iter()
        .max_by_key(|(bw, _)| *bw)
        .map(|(_, url)| url.clone())
        .unwrap_or_else(|| base_url.to_string())
}

pub fn segment_uris(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// 把上游分片 URL 变成安全的本地文件名。
pub fn local_segment_name(url: &str) -> String {
    let path = Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| url.split('?').next().unwrap_or(url).to_string());
    let base = path.rsplit('/').next().unwrap_or("seg.ts");
    let mut name: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        name = "seg.ts".into();
    }
    if !name.ends_with(".ts") {
        name.push_str(".ts");
    }
    name
}

/// 把分片 URI 换成本地路径。
///
/// `register(abs_url, suggested_name) -> final_name` 由调用方维护 URL 表。
/// 返回 (新播放列表, 最后一个分片名)。
pub fn rewrite<F>(
    text: &str,
    media_url: &str,
    prefix: &str,
    mut register: F,
) -> (String, Option<String>)
where
    F: FnMut(&str, String) -> String,
{
    let mut out = String::with_capacity(text.len());
    let mut last: Option<String> = None;
    for line in text.lines() {
        let stripped = line.trim();
        if stripped.is_empty() {
            continue;
        }
        if stripped.starts_with('#') {
            out.push_str(stripped);
            out.push('\n');
            continue;
        }
        let abs_url = join_url(media_url, stripped);
        let name = local_segment_name(&abs_url);
        let name = register(&abs_url, name);
        last = Some(name.clone());
        out.push_str(prefix);
        out.push_str(&name);
        out.push('\n');
    }
    (out, last)
}

pub const CODECS_HINT: &str = "avc1.640028,mp4a.40.2";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_highest_variant() {
        let master = "#EXTM3U\n\
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=1280x720\n720.m3u8\n\
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1920x1080\n1080.m3u8\n";
        let base = "https://cdn.example.com/a/b/index.m3u8?token=1";
        assert_eq!(
            pick_variant(master, base, 0),
            "https://cdn.example.com/a/b/1080.m3u8"
        );
        assert_eq!(
            pick_variant(master, base, 1_000_000),
            "https://cdn.example.com/a/b/720.m3u8"
        );
    }

    #[test]
    fn rewrite_relative_segments() {
        let text = "#EXTM3U\n#EXTINF:5.0,\nseg-1.ts?x=1\n#EXTINF:5.0,\nseg-2.ts\n";
        let mut urls = Vec::new();
        let (out, last) = rewrite(
            text,
            "https://cdn.example.com/live/index.m3u8?token=1",
            "/live/cctv1/",
            |abs, name| {
                urls.push(abs.to_string());
                name
            },
        );
        assert!(out.contains("/live/cctv1/seg-1.ts\n"));
        assert_eq!(last.as_deref(), Some("seg-2.ts"));
        assert_eq!(urls[0], "https://cdn.example.com/live/seg-1.ts?x=1");
    }

    #[test]
    fn local_names_are_safe() {
        assert_eq!(local_segment_name("https://x/y/a:b.ts?q=1"), "a_b.ts");
        assert_eq!(local_segment_name("https://x/y/no_ext"), "no_ext.ts");
    }
}
