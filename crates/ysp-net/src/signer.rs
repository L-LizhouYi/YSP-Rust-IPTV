//! yspsdksign 提供者：h5access token + 一次 md5（官方 wasm 的纯 Rust 等价实现）。

use ysp_core::crypto::sign::sdk_signature;

use crate::{error::NetError, http::HttpClient, token::TokenClient};

pub struct Signer {
    tokens: TokenClient,
}

impl Signer {
    pub fn new(http: HttpClient) -> Self {
        Self {
            tokens: TokenClient::new(http),
        }
    }

    pub async fn sign(&self, sign_input: &str, guid: &str) -> Result<String, NetError> {
        let token = self.tokens.token(guid, ysp_core::config::YSP_APPID).await?;
        Ok(sdk_signature(sign_input, guid, &token))
    }
}
