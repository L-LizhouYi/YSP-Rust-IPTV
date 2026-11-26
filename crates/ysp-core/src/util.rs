//! 无副作用的工具函数。

use chrono::{FixedOffset, Utc};
use rand::Rng;

const RAND_CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

pub fn md5_hex(text: &[u8]) -> String {
    format!("{:x}", md5::compute(text))
}

/// 10 位字母数字（等价 Python `random_alpha`）。
pub fn random_alpha(n: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..n).map(|_| RAND_CHARS[rng.gen_range(0..RAND_CHARS.len())] as char).collect()
}

/// n 字节随机数的 hex（h5access 的 rnd 是 32 位 hex = 16 字节）。
pub fn random_hex(n_bytes: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..n_bytes).map(|_| format!("{:02x}", rng.gen::<u8>())).collect()
}

pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn now_secs() -> i64 {
    (now_ms() / 1000) as i64
}

/// JS 32 位有符号 djb2（与原站 cKey 内联实现逐位一致）。
pub fn js_djb2(text: &str) -> i32 {
    let mut h: u32 = 0;
    for ch in text.chars() {
        h = h.wrapping_shl(5).wrapping_sub(h).wrapping_add(ch as u32);
    }
    h as i32
}

/// 北京时间（央视频节目单按 +0800 切日）。
pub fn cn_offset() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("UTC+8 合法")
}

pub fn today_compact() -> String {
    Utc::now().with_timezone(&cn_offset()).format("%Y%m%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn djb2_matches_js() {
        assert_eq!(js_djb2(""), 0);
        assert_eq!(js_djb2("a"), 97);
        // Python: js_djb2(向量里的 Sl 前缀) == -965908217
        assert_eq!(js_djb2("|2029797201|1795241410|mg3c3b04ba|V1.0.0|mwpvxhkj_wotngaq1q1|5910204|https://www.yangshipin.c|mozilla/5.0 (macintosh; ||Mozilla|Netscape|MacIntel|"), -965908217);
    }
}
