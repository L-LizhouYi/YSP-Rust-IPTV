//! 播放器接口客户端：auth + get_live_info。

use std::collections::BTreeMap;

use serde_json::Value;
use ysp_core::{
    config,
    crypto::{ckey::make_ckey, sign, ticket::make_ticket},
    models::{AuthResult, Channel, Format, PlayInfo},
    util::{now_ms, now_secs, random_alpha},
};

use crate::{
    error::NetError,
    http::{header, Header, HttpClient},
    signer::Signer,
};

const LIVE_INFO_MAX: usize = 1024 * 1024;
const AUTH_MAX: usize = 256 * 1024;

pub struct Player {
    pub http: HttpClient,
    pub signer: Signer,
    pub guid: String,
}

impl Player {
    pub fn new(http: HttpClient, signer: Signer, guid: impl Into<String>) -> Self {
        Self {
            http,
            signer,
            guid: guid.into(),
        }
    }

    fn default_cookie(&self) -> String {
        format!(
            "ysp_uv=1; guid={}; versionName=99.99.99; versionCode=999999; \
vplatform=109; platformVersion=Chrome; deviceModel=152; newLogin=1; pc_version=1.1.16",
            self.guid
        )
    }

    fn headers(&self, extra: Vec<Header>) -> Vec<Header> {
        let mut headers = vec![
            header("User-Agent", config::USER_AGENT),
            header("Referer", config::REFERER),
            header("Origin", config::ORIGIN),
            header("yspappid", config::YSP_APPID),
            header("Cookie", self.default_cookie()),
        ];
        headers.extend(extra);
        headers
    }

    fn parse_json(text: &str, what: &str) -> Result<Value, NetError> {
        serde_json::from_str(text).map_err(|_| {
            NetError::Parse(format!(
                "{what} 返回非 JSON: {}",
                &text[..text.len().min(200)]
            ))
        })
    }

    /// 获取 yspplayertoken（按频道 pid 签发）。
    pub async fn auth(&self, livepid: &str) -> Result<AuthResult, NetError> {
        let rand = random_alpha(10);
        let body = sign::auth_body(livepid, &self.guid, &rand);
        let payload = format!("{body}&signature={}", sign::auth_signature(&body));
        let text = self
            .http
            .post_form(&config::AUTH_URL, &payload, &self.headers(vec![]), AUTH_MAX)
            .await?;
        let data = Self::parse_json(&text, "auth")?;
        let code = data.get("code").and_then(Value::as_i64).unwrap_or(-1);
        let inner = data.get("data").cloned().unwrap_or(Value::Null);
        let token = inner.get("token").and_then(Value::as_str).unwrap_or("");
        if code != 0 || token.is_empty() {
            return Err(NetError::Upstream {
                code,
                msg: format!("auth 失败: {}", &text[..text.len().min(200)]),
            });
        }
        Ok(AuthResult {
            token: token.to_string(),
            ts: inner
                .get("ts")
                .map(|v| v.to_string().trim_matches('"').to_string())
                .unwrap_or_default(),
        })
    }

    fn build_payload(
        &self,
        channel: &Channel,
        defn: &str,
        ts: i64,
    ) -> BTreeMap<String, String> {
        let mut payload = BTreeMap::new();
        payload.insert("cnlid".into(), channel.cnlid.clone());
        payload.insert("livepid".into(), channel.livepid.clone());
        payload.insert("stream".into(), "2".into());
        payload.insert("guid".into(), self.guid.clone());
        payload.insert("cKey".into(), make_ckey(&channel.cnlid, ts, &self.guid));
        payload.insert("adjust".into(), "1".into());
        payload.insert("sphttps".into(), "1".into());
        payload.insert("platform".into(), config::PLATFORM.into());
        payload.insert("cmd".into(), "2".into());
        payload.insert("encryptVer".into(), "8.1".into());
        payload.insert("dtype".into(), "1".into());
        payload.insert("devid".into(), "devid".into());
        payload.insert("otype".into(), "ojson".into());
        payload.insert("appVer".into(), config::APP_VER.into());
        payload.insert("app_version".into(), config::APP_VER.into());
        payload.insert("channel".into(), config::CHANNEL_TAG.into());
        payload.insert("defn".into(), defn.into());
        payload
    }

    /// 请求播放地址；`playurl` 为空表示不可播（VIP 频道）。
    pub async fn live_info(&self, channel: &Channel, defn: &str) -> Result<PlayInfo, NetError> {
        let auth = self.auth(&channel.livepid).await?;
        let payload = self.build_payload(channel, defn, now_secs());

        let sdk_in = sign::sdk_input(&payload);
        let seq = now_secs() % 90 + 1;
        let request_id = format!("999999{}{}", random_alpha(10), now_ms());
        let sign_input = sign::sdk_sign_input(&sdk_in, &self.guid, seq, &request_id);
        let sdk_sign = self.signer.sign(&sign_input, &self.guid).await?;

        let mut signed = payload.clone();
        let rand_str = random_alpha(10);
        signed.insert("rand_str".into(), rand_str.clone());
        let signature = sign::body_signature(&signed);

        let mut obj = serde_json::Map::new();
        for (k, v) in payload.iter() {
            let value = if k == "adjust" {
                serde_json::json!(1)
            } else {
                Value::String(v.clone())
            };
            obj.insert(k.clone(), value);
        }
        obj.insert("rand_str".into(), Value::String(rand_str));
        obj.insert("signature".into(), Value::String(signature));

        let ticket = make_ticket(
            &channel.livepid,
            &auth.ts,
            &self.guid,
            None,
        )
        .map_err(|e| NetError::Sign(e.to_string()))?;

        let headers = self.headers(vec![
            header("seqid", seq.to_string()),
            header("request-id", request_id),
            header("yspplayertoken", auth.token.clone()),
            header("yspsdkinput", sdk_in),
            header("yspsdksign", sdk_sign),
            header("yspticket", ticket),
        ]);

        let text = self
            .http
            .post_json(
                &config::LIVE_INFO_URL,
                &Value::Object(obj),
                &headers,
                LIVE_INFO_MAX,
            )
            .await?;
        let data = Self::parse_json(&text, "get_live_info")?;
        let code = data.get("code").and_then(Value::as_i64).unwrap_or(-1);
        if code != 0 {
            return Err(NetError::Upstream {
                code,
                msg: format!(
                    "get_live_info 失败: code={} msg={}",
                    code,
                    data.get("msg").and_then(Value::as_str).unwrap_or("")
                ),
            });
        }

        let info = data.get("data").cloned().unwrap_or(Value::Null);
        let formats = info
            .get("formats")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| {
                        let fn_ = f.get("fn").and_then(Value::as_str)?;
                        if fn_.is_empty() {
                            return None;
                        }
                        let label = f
                            .get("defnrate")
                            .and_then(Value::as_str)
                            .or_else(|| f.get("defnname").and_then(Value::as_str))
                            .unwrap_or("");
                        Some(Format {
                            fn_: fn_.to_string(),
                            label: label.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(PlayInfo {
            playurl: info
                .get("playurl")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            defn: info
                .get("defn")
                .and_then(Value::as_str)
                .unwrap_or(defn)
                .to_string(),
            // 官方当前对 fhd/shd/hd 都返回 encrypt=2；缺省按加密处理。
            encrypt: info.get("encrypt").and_then(Value::as_i64).unwrap_or(2),
            formats,
            cdn: info
                .get("cdn_name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            livepid: channel.livepid.clone(),
            cnlid: channel.cnlid.clone(),
            auth_ts: auth.ts,
        })
    }

    /// 按清晰度优先级尝试，直到拿到 playurl。
    pub async fn pick_playable(&self, channel: &Channel) -> Result<PlayInfo, NetError> {
        let mut last: Option<PlayInfo> = None;
        let mut errors = Vec::new();
        for defn in ["fhd", "shd", "hd"] {
            match self.live_info(channel, defn).await {
                Ok(info) => {
                    if !info.playurl.is_empty() {
                        return Ok(info);
                    }
                    last = Some(info);
                }
                Err(err) => errors.push(format!("{defn}: {err}")),
            }
        }
        if last.is_some() {
            return Err(NetError::NotPlayable {
                msg: format!("{} 暂不可播（VIP/版权限制）", channel.name),
                code: 54,
            });
        }
        Err(NetError::Upstream {
            code: 0,
            msg: format!("全部清晰度请求失败: {}", errors.join("; ")),
        })
    }
}
