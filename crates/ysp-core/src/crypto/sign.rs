//! 各类签名：auth body / live body / yspsdkinput / yspsdksign。

use std::collections::BTreeMap;

use percent_encoding::percent_decode_str;

use crate::{config, util::md5_hex};

/// 普通字典序 `k=v&...`。
fn sorted_kv(pairs: &[(&str, &str)]) -> String {
    let mut items: Vec<(&str, &str)> = pairs.to_vec();
    items.sort_by(|a, b| a.0.cmp(b.0));
    items
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn auth_body(livepid: &str, guid: &str, rand: &str) -> String {
    sorted_kv(&[
        ("pid", livepid),
        ("guid", guid),
        ("appid", "ysp_pc"),
        ("rand_str", rand),
    ])
}

pub fn auth_signature(body: &str) -> String {
    md5_hex(format!("{}{}", body, config::AUTH_KEY).as_bytes())
}

pub fn body_signature(payload: &BTreeMap<String, String>) -> String {
    let body = payload
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    md5_hex(format!("{}{}", body, config::LIVE_BODY_KEY).as_bytes())
}

/// yspsdkinput：键按 JS `localeCompare` 排序（对本接口等价于按小写排序），
/// 值先 percent-decode 再拼接。
pub fn sdk_input(payload: &BTreeMap<String, String>) -> String {
    let mut items: Vec<(&String, &String)> = payload.iter().collect();
    items.sort_by_key(|(k, _)| k.to_lowercase());
    let joined = items
        .iter()
        .map(|(k, v)| {
            let decoded = percent_decode_str(v).decode_utf8_lossy();
            format!("{k}={decoded}")
        })
        .collect::<Vec<_>>()
        .join("&");
    md5_hex(joined.as_bytes())
}

pub fn sdk_sign_input(sdk_in: &str, guid: &str, seq: i64, request_id: &str) -> String {
    format!("{sdk_in}-{guid}-{seq}-{request_id}")
}

/// yspsdksign = `md5(guid;host;token;protocol;yspappid;input;)` + `-input`
pub fn sdk_signature(sign_input: &str, guid: &str, token: &str) -> String {
    let raw = format!(
        "{};{};{};{};{};{};",
        guid,
        config::SIGN_HOST,
        token,
        config::SIGN_PROTOCOL,
        config::YSP_APPID,
        sign_input
    );
    format!("{}-{}", md5_hex(raw.as_bytes()), sign_input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_body() -> BTreeMap<String, String> {
        // tests/vectors.py 的 LIVE_BODY
        let mut m = BTreeMap::new();
        for (k, v) in [
            ("cnlid", "2029797201"),
            ("livepid", "600001811"),
            ("stream", "2"),
            ("guid", "mwpvxhkj_wotngaq1q1"),
            ("cKey", "--0182D3691E5D34046399D948EF03183842CE6A50F3A4E278AA112C1CCCDD0D4981C4FE81147527F95A55F57CF81A28AC0DDB3E817D5F2E4D657C4D16A96DF598F54091FAE6C4421CE6D688599E8D311338D255EE987D6B0910F2AE5953ACB0B3D8CFF3FE4139290B8235F29CABA0D5C6F76134F9CC0E60C5842AED5CE3C753887019024E281C9B3FCF36F55D231DF1C83F45CA216CCB6535CAA3CBB3702B464792"),
            ("adjust", "1"),
            ("sphttps", "1"),
            ("platform", "5910204"),
            ("cmd", "2"),
            ("encryptVer", "8.1"),
            ("dtype", "1"),
            ("devid", "devid"),
            ("otype", "ojson"),
            ("appVer", "V1.0.0"),
            ("app_version", "V1.0.0"),
            ("channel", "ysp_tx"),
            ("defn", "fhd"),
        ] {
            m.insert(k.to_string(), v.to_string());
        }
        m
    }

    #[test]
    fn auth_body_and_signature() {
        let body = auth_body("600001811", "mwpvxhkj_wotngaq1q1", "TuAeaURlmV");
        assert_eq!(
            body,
            "appid=ysp_pc&guid=mwpvxhkj_wotngaq1q1&pid=600001811&rand_str=TuAeaURlmV"
        );
        assert_eq!(auth_signature(&body), "1f9415560d1473f0f2249891b2f16166");
    }

    #[test]
    fn sdk_input_vector() {
        assert_eq!(sdk_input(&live_body()), "8430706eb68f036f3dbebe47cd12535e");
    }

    #[test]
    fn body_signature_vector() {
        let mut body = live_body();
        body.insert("rand_str".into(), "wnp62p6cl8".into());
        assert_eq!(body_signature(&body), "b4c313eacbc843ec11d97aed2165ce85");
    }

    #[test]
    fn sdk_signature_vectors() {
        assert_eq!(
            sdk_signature("abc", "mwpvxhkj_wotngaq1q1",
                          "35e0dd08ef3d07a08e90c40e911018bc"),
            "9484406373b8d37be0ba22b4d2a39441-abc"
        );
        let long = "8430706eb68f036f3dbebe47cd12535e-mwpvxhkj_wotngaq1q1-7-req";
        assert_eq!(
            sdk_signature(long, "mwpvxhkj_wotngaq1q1",
                          "35e0dd08ef3d07a08e90c40e911018bc"),
            format!("fb199eff2fa992106afb34bd58e35317-{long}")
        );
    }
}
