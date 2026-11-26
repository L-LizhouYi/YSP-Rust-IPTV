//! 领域模型（纯数据）。序列化字段与 Python 版 `/api/channels` 保持一致。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn default_category() -> String {
    "other".into()
}
fn default_tier() -> String {
    "free".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub livepid: String,
    pub cnlid: String,
    pub name: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_tier")]
    pub tier: String,
    #[serde(default)]
    pub logo: String,
    /// URL 用的稳定 ID。目录加载后由 `assign_ids` 统一定稿。
    #[serde(default)]
    pub id: String,
}

impl Channel {
    pub fn new(livepid: String, cnlid: String, name: String, category: String,
               tier: String, logo: String) -> Self {
        let mut ch = Channel { livepid, cnlid, name, category, tier, logo, id: String::new() };
        ch.id = ch.base_slug();
        ch
    }

    /// 与 Python `slug` 相同：ASCII 名称（<=12 字符）优先，否则用 livepid。
    pub fn base_slug(&self) -> String {
        let lowered = self.name.to_lowercase();
        let ascii: String = lowered
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '+')
            .collect();
        if !ascii.is_empty() && ascii.len() <= 12 {
            ascii
        } else {
            self.livepid.clone()
        }
    }

    pub fn id(&self) -> &str {
        if self.id.is_empty() {
            &self.livepid
        } else {
            &self.id
        }
    }

    pub fn to_json(&self, playable: Option<bool>, defn: &str, formats: &[Format]) -> Value {
        json!({
            "id": self.id(),
            "livepid": self.livepid,
            "cnlid": self.cnlid,
            "name": self.name,
            "category": self.category,
            "tier": self.tier,
            "logo": self.logo,
            "playable": playable,
            "defn": defn,
            "formats": formats,
        })
    }
}

/// 给所有频道分配唯一 ID。重名（例如三个 CGTN 频道都会被裁成 `cgtn`）
/// 时后来者退回 livepid，保证 `/live/{id}` 不会串台。
pub fn assign_ids(channels: &mut [Channel]) {
    let mut used = std::collections::HashSet::new();
    for ch in channels.iter_mut() {
        let mut id = ch.base_slug();
        if used.contains(&id) {
            id = ch.livepid.clone();
        }
        used.insert(id.clone());
        ch.id = id;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub program_id: String,
    pub name: String,
    pub start_ts: i64,
    pub end_ts: i64,
    #[serde(default)]
    pub start_time: String,
    #[serde(default)]
    pub end_time: String,
}

impl Program {
    pub fn duration(&self) -> i64 {
        (self.end_ts - self.start_ts).max(0)
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.program_id,
            "name": self.name,
            "start": self.start_ts,
            "end": self.end_ts,
            "start_time": self.start_time,
            "end_time": self.end_time,
            "duration": self.duration(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Format {
    #[serde(rename = "fn")]
    pub fn_: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlayInfo {
    pub playurl: String,
    pub defn: String,
    #[serde(default)]
    pub encrypt: i64,
    #[serde(default)]
    pub formats: Vec<Format>,
    #[serde(default)]
    pub cdn: String,
    #[serde(default)]
    pub livepid: String,
    #[serde(default)]
    pub cnlid: String,
    #[serde(default)]
    pub auth_ts: String,
}

impl PlayInfo {
    pub fn to_json(&self) -> Value {
        json!({
            "playurl": self.playurl,
            "defn": self.defn,
            "encrypt": self.encrypt,
            "formats": self.formats,
            "cdn": self.cdn,
            "livepid": self.livepid,
            "cnlid": self.cnlid,
            "auth_ts": self.auth_ts,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuthResult {
    pub token: String,
    pub ts: String,
}
