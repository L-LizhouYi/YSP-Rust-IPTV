//! TS 解密门面：根据线路选择 enc2 / h5e。

pub mod enc2;
pub mod h5e;
pub mod playlist;
pub mod tea;
pub mod ts;

use crate::config::DEFAULT_VIDEO_PID;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecryptMode {
    /// encrypt=2：1080P/720P/540P 通用的 TEA 网格。
    Enc2,
    /// encrypt=1：老 h5e classic 网格（保留兼容）。
    H5e,
    #[default]
    None,
}

impl DecryptMode {
    /// 官方 `get_live_info` 的 `encrypt` 字段：2 -> enc2，1 -> h5e，0 -> 明文。
    /// 未知值按加密处理，避免把密文直接吐给播放器。
    pub fn from_encrypt(value: i64) -> Self {
        match value {
            0 => DecryptMode::None,
            1 => DecryptMode::H5e,
            _ => DecryptMode::Enc2,
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "enc2" | "2" => Some(DecryptMode::Enc2),
            "h5e" | "1" => Some(DecryptMode::H5e),
            "none" | "0" => Some(DecryptMode::None),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DecryptMode::Enc2 => "enc2",
            DecryptMode::H5e => "h5e",
            DecryptMode::None => "none",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub nals: usize,
    pub decrypted: usize,
    pub skipped: usize,
    pub cells: usize,
    pub pes: usize,
    pub delta: i64,
    pub epb_dropped: usize,
}

/// 解密一个 TS 分片。`None` 模式原样返回（不复制）。
pub fn decrypt(mode: DecryptMode, data: &[u8]) -> (Vec<u8>, Stats) {
    match mode {
        DecryptMode::None => (data.to_vec(), Stats::default()),
        DecryptMode::Enc2 => enc2::decrypt_ts(data, None),
        DecryptMode::H5e => h5e::decrypt_ts(data, None),
    }
}

/// 显式指定视频 PID（调试用）。
pub fn decrypt_with_pid(mode: DecryptMode, data: &[u8], pid: Option<u16>) -> (Vec<u8>, Stats) {
    match mode {
        DecryptMode::None => (data.to_vec(), Stats::default()),
        DecryptMode::Enc2 => enc2::decrypt_ts(data, pid.or(Some(DEFAULT_VIDEO_PID))),
        DecryptMode::H5e => h5e::decrypt_ts(data, pid),
    }
}
