use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum NetError {
    #[error("网络错误: {0}")]
    Transport(String),

    #[error("HTTP {status}: {body}")]
    HttpStatus {
        status: u16,
        url: String,
        body: String,
    },

    #[error("响应体超过上限: {0}")]
    TooLarge(String),

    #[error("上游返回 code={code}: {msg}")]
    Upstream { code: i64, msg: String },

    #[error("频道暂不可播: {msg}")]
    NotPlayable { msg: String, code: i64 },

    #[error("签名失败: {0}")]
    Sign(String),

    #[error("解析失败: {0}")]
    Parse(String),
}

impl NetError {
    pub(crate) fn retryable(&self) -> bool {
        match self {
            NetError::Transport(_) => true,
            NetError::HttpStatus { status, .. } => *status >= 500,
            _ => false,
        }
    }

    /// 映射到对外的 HTTP 状态码。
    pub fn http_status(&self) -> u16 {
        match self {
            NetError::NotPlayable { .. } => 409,
            NetError::HttpStatus { .. } | NetError::Upstream { .. } | NetError::Transport(_) => 502,
            NetError::TooLarge(_) => 502,
            NetError::Parse(_) | NetError::Sign(_) => 500,
        }
    }
}
