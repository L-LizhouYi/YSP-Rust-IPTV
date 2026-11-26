use thiserror::Error;

/// 核心层错误。`code` 保留上游接口的整数码，便于 HTTP 层映射。
#[derive(Debug, Clone, Error)]
pub enum YspError {
    #[error("上游返回 code={code}: {msg}")]
    Upstream { code: i64, msg: String },

    #[error("频道暂不可播: {msg}")]
    NotPlayable { msg: String, code: i64 },

    #[error("协议解析失败: {0}")]
    Protocol(String),

    #[error("加解密失败: {0}")]
    Crypto(String),
}

impl YspError {
    pub fn upstream_code(&self) -> i64 {
        match self {
            YspError::Upstream { code, .. } => *code,
            YspError::NotPlayable { code, .. } => *code,
            _ => 0,
        }
    }
}
