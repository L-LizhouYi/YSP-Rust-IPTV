//! cKey 生成（逆向自官方播放器混淆代码里的 `sc()`）。

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};

use crate::{config, util::js_djb2};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

fn head24(s: &str) -> String {
    s.chars().take(24).collect()
}

/// Sl = "|" + cnlid|ts|mg3c3b04ba|appVer|guid|platform|
///             href[:24]|ua.lower()[:24]|localStorage|
///             appCodeName|appName|platformName|"
///
/// plaintext = "|" + djb2(Sl) + Sl
pub fn build_plaintext(cnlid: &str, ts: i64, guid: &str) -> Vec<u8> {
    let sl = format!(
        "|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|",
        cnlid,
        ts,
        config::CKEY_SALT,
        config::APP_VER,
        guid,
        config::PLATFORM,
        head24(config::PAGE_URL),
        head24(&config::USER_AGENT.to_lowercase()),
        "",
        "Mozilla",
        "Netscape",
        "MacIntel",
    );
    format!("|{}{}", js_djb2(&sl), sl).into_bytes()
}

/// 返回 `--01` + hex(AES-128-CBC-PKCS7(plaintext))。
pub fn make_ckey(cnlid: &str, ts: i64, guid: &str) -> String {
    let plaintext = build_plaintext(cnlid, ts, guid);
    let cipher = Aes128CbcEnc::new(
        (&config::CKEY_AES_KEY).into(),
        (&config::CKEY_AES_IV).into(),
    );
    let ciphertext = cipher.encrypt_padded_vec_mut::<Pkcs7>(&plaintext);
    format!("{}{}", config::CKEY_PREFIX, hex::encode(ciphertext))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 真实浏览器抓包（tests/vectors.py）
    const CNLID: &str = "2029797201";
    const TS: i64 = 1795241410;
    const GUID: &str = "mwpvxhkj_wotngaq1q1";
    const EXPECTED: &str = "--0182D3691E5D34046399D948EF03183842CE6A50F3A4E278AA112C1CCCDD0D4981C4FE81147527F95A55F57CF81A28AC0DDB3E817D5F2E4D657C4D16A96DF598F54091FAE6C4421CE6D688599E8D311338D255EE987D6B0910F2AE5953ACB0B3D8CFF3FE4139290B8235F29CABA0D5C6F76134F9CC0E60C5842AED5CE3C753887019024E281C9B3FCF36F55D231DF1C83F45CA216CCB6535CAA3CBB3702B464792";

    #[test]
    fn plaintext_layout() {
        let text = String::from_utf8(build_plaintext(CNLID, TS, GUID)).unwrap();
        assert_eq!(
            text,
            "|-965908217|2029797201|1795241410|mg3c3b04ba|V1.0.0|\
mwpvxhkj_wotngaq1q1|5910204|https://www.yangshipin.c|\
mozilla/5.0 (macintosh; ||Mozilla|Netscape|MacIntel|"
        );
    }

    #[test]
    fn ckey_matches_browser() {
        assert_eq!(make_ckey(CNLID, TS, GUID).to_lowercase(), EXPECTED.to_lowercase());
    }
}
