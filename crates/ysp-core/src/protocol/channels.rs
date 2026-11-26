//! 频道目录解析（官方页面配置 PG00000004，protobuf）。
//!
//! 消息 #{15} 一个频道：
//!   #1 "tv-channel-list" / #2 频道名 / #4 livepid / #5 logo / #6 cnlid
//!   #11 分类：yangshi(CCTV) / weishi(卫视)
//!   #16 付费标签（嵌套消息里带 “电视-pid-VIP” / “电视-pid-限免”）

use std::collections::HashSet;

use crate::{
    models::{assign_ids, Channel},
    protocol::protobuf::{parse, text_field, walk, Field, Value, WIRE_LEN},
};

const CHANNEL_FIELD: u32 = 15;

fn is_livepid(text: &str) -> bool {
    (6..=12).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

fn tier_of(entry: &[Field<'_>]) -> String {
    for field in entry {
        if field.number != 16 || field.wire != WIRE_LEN {
            continue;
        }
        let Value::Bytes(body) = field.value else { continue };
        let mut hit: Option<&'static str> = None;
        walk(body, &mut |sub| {
            if hit.is_some() {
                return;
            }
            for item in sub {
                if item.wire != WIRE_LEN {
                    continue;
                }
                let text = item.as_text();
                if text.contains("电视-pid-VIP") {
                    hit = Some("vip");
                } else if text.contains("电视-pid-限免") {
                    hit = Some("preview");
                }
            }
        });
        if let Some(tier) = hit {
            return tier.to_string();
        }
    }
    "free".to_string()
}

/// 从页面配置里提取全部频道（按出现顺序，livepid 去重）。
pub fn parse_channels(buf: &[u8]) -> Vec<Channel> {
    let mut channels: Vec<Channel> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    walk(buf, &mut |fields| {
        for field in fields {
            if field.number != CHANNEL_FIELD || field.wire != WIRE_LEN {
                continue;
            }
            let Ok(entry) = parse(field.as_bytes()) else { continue };
            let (Some(name), Some(livepid), Some(cnlid)) = (
                text_field(&entry, 2),
                text_field(&entry, 4),
                text_field(&entry, 6),
            ) else {
                continue;
            };
            if !is_livepid(&livepid) || !is_livepid(&cnlid) {
                continue;
            }
            if !seen.insert(livepid.clone()) {
                continue;
            }
            let category = text_field(&entry, 11).unwrap_or_else(|| "other".into());
            let logo = text_field(&entry, 13)
                .or_else(|| text_field(&entry, 5))
                .unwrap_or_default();
            channels.push(Channel::new(
                livepid,
                cnlid,
                name,
                category,
                tier_of(&entry),
                logo,
            ));
        }
    });

    assign_ids(&mut channels);
    channels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            out.push(byte | if value != 0 { 0x80 } else { 0 });
            if value == 0 {
                return out;
            }
        }
    }
    fn f_varint(number: u32, value: u64) -> Vec<u8> {
        let mut out = varint(((number as u64) << 3) | 0);
        out.extend(varint(value));
        out
    }
    fn f_bytes(number: u32, data: &[u8]) -> Vec<u8> {
        let mut out = varint(((number as u64) << 3) | 2);
        out.extend(varint(data.len() as u64));
        out.extend_from_slice(data);
        out
    }
    fn f_text(number: u32, text: &str) -> Vec<u8> {
        f_bytes(number, text.as_bytes())
    }

    fn channel_entry(name: &str, livepid: &str, cnlid: &str, category: &str,
                     tier_text: &str) -> Vec<u8> {
        let mut body = f_text(1, "tv-channel-list");
        body.extend(f_text(2, name));
        body.extend(f_text(4, livepid));
        body.extend(f_text(5, &format!("https://logo/{livepid}.png")));
        body.extend(f_text(6, cnlid));
        body.extend(f_text(11, category));
        if !tier_text.is_empty() {
            body.extend(f_bytes(16, &f_text(1, tier_text)));
        }
        body.extend(f_text(13, &format!("https://logo13/{livepid}.png")));
        body
    }

    fn build_page(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut inner = f_text(13, "tv_channel_list");
        for entry in entries {
            inner.extend(f_bytes(15, entry));
        }
        let mut page = f_varint(1, 200);
        let mut sub = f_text(1, "PG00000004");
        sub.extend(f_bytes(2, &inner));
        page.extend(f_bytes(2, &sub));
        page
    }

    #[test]
    fn parse_tiers_and_slug() {
        let page = build_page(&[
            channel_entry("CCTV13", "600001811", "2029797201", "yangshi", ""),
            channel_entry("湖南卫视", "600002475", "2024054803", "weishi", ""),
            channel_entry("CCTV世界地理频道", "600099637", "2026874403", "yangshi",
                          "电视-pid-VIP"),
            channel_entry("CCTV风云剧场频道", "600099658", "2025637103", "yangshi",
                          "电视-pid-限免"),
        ]);
        let channels = parse_channels(&page);
        assert_eq!(channels.len(), 4);
        assert_eq!(channels[0].livepid, "600001811");
        assert_eq!(channels[0].id(), "cctv13");
        assert_eq!(channels[1].category, "weishi");
        assert_eq!(channels[2].tier, "vip");
        assert_eq!(channels[3].tier, "preview");
        assert!(channels[0].logo.ends_with(".png"));
    }

    #[test]
    fn dedup_by_livepid() {
        let entry = channel_entry("CCTV1", "600001859", "2024078201", "yangshi", "");
        let page = build_page(&[entry.clone(), entry]);
        assert_eq!(parse_channels(&page).len(), 1);
    }

    #[test]
    fn duplicate_slugs_fall_back_to_livepid() {
        let page = build_page(&[
            channel_entry("CGTN", "600001888", "2025000001", "yangshi", ""),
            channel_entry("CGTN阿拉伯语频道", "600001889", "2025000002", "yangshi", ""),
        ]);
        let channels = parse_channels(&page);
        assert_eq!(channels[0].id(), "cgtn");
        assert_eq!(channels[1].id(), "600001889");
    }
}
