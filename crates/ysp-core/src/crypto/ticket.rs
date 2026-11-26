//! yspticket：官方固定 keystream 的 AES-CTR 等价实现（直接异或）。
//!
//! 明文 = `{livepid}&{ts}&{guid}&{YSP_APPID}&{10位随机}`，长度必须为 61。

use crate::{config, error::YspError, util::random_alpha};

pub fn build_plaintext(livepid: &str, ts: &str, guid: &str, nonce: &str) -> Vec<u8> {
    format!("{}&{}&{}&{}&{}", livepid, ts, guid, config::YSP_APPID, nonce).into_bytes()
}

pub fn make_ticket(
    livepid: &str,
    ts: &str,
    guid: &str,
    nonce: Option<&str>,
) -> Result<String, YspError> {
    let nonce = nonce.map(str::to_string).unwrap_or_else(|| random_alpha(10));
    let plain = build_plaintext(livepid, ts, guid, &nonce);
    if plain.len() != config::TICKET_KEYSTREAM.len() {
        return Err(YspError::Crypto(format!(
            "yspticket 明文长度 {} 与 keystream {} 不一致（livepid 需为 9 位）",
            plain.len(),
            config::TICKET_KEYSTREAM.len()
        )));
    }
    let out: Vec<u8> = plain
        .iter()
        .zip(config::TICKET_KEYSTREAM.iter())
        .map(|(a, b)| a ^ b)
        .collect();
    Ok(hex::encode(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_matches_vector() {
        let ticket = make_ticket("600001811", "1795241335", "mwpvxhkj_wotngaq1q1",
                                 Some("4qmaYWJIYy")).unwrap();
        let raw = hex::decode(ticket).unwrap();
        let plain: Vec<u8> = raw
            .iter()
            .zip(config::TICKET_KEYSTREAM.iter())
            .map(|(a, b)| a ^ b)
            .collect();
        assert_eq!(
            String::from_utf8(plain).unwrap(),
            "600001811&1795241335&mwpvxhkj_wotngaq1q1&519748109&4qmaYWJIYy"
        );
        assert_eq!(raw.len(), 61);
    }
}
