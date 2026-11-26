//! `/channels.m3u` 生成。
//!
//! 输出格式（按需求固定）：
//! ```text
//! #EXTM3U url-tvg="http://host/epg.xml"
//! #EXTINF:-1 tvg-id="cctv1" tvg-name="CCTV1" tvg-logo="..." group-title="央视" ,CCTV1
//! http://host/live/cctv1.m3u8
//! ```

use ysp_core::models::Channel;

fn escape_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn group_title(category: &str) -> &'static str {
    match category {
        "yangshi" => "央视",
        "weishi" => "卫视",
        _ => "其他",
    }
}

pub fn render(channels: &[Channel], base: &str) -> String {
    let mut out = String::with_capacity(channels.len() * 180 + 64);
    out.push_str(&format!("#EXTM3U url-tvg=\"{}/epg.xml\"\n", base));
    for ch in channels {
        out.push_str(&format!(
            "#EXTINF:-1 tvg-id=\"{}\" tvg-name=\"{}\" tvg-logo=\"{}\" group-title=\"{}\" ,{}\n",
            escape_attr(ch.id()),
            escape_attr(&ch.name),
            escape_attr(&ch.logo),
            group_title(&ch.category),
            ch.name
        ));
        out.push_str(&format!("{}/live/{}.m3u8\n", base, ch.id()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m3u_shape() {
        let ch = Channel::new(
            "600001811".into(),
            "2029797201".into(),
            "CCTV1".into(),
            "yangshi".into(),
            "free".into(),
            "https://logo/cctv1.png".into(),
        );
        let text = render(&[ch], "http://10.1.10.3:8525");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "#EXTM3U url-tvg=\"http://10.1.10.3:8525/epg.xml\"");
        assert_eq!(
            lines[1],
            "#EXTINF:-1 tvg-id=\"cctv1\" tvg-name=\"CCTV1\" \
tvg-logo=\"https://logo/cctv1.png\" group-title=\"央视\" ,CCTV1"
        );
        assert_eq!(lines[2], "http://10.1.10.3:8525/live/cctv1.m3u8");
    }
}
