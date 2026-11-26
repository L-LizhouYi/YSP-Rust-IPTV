//! 全部「魔数」。与 Python 版 `ysp/config.py` 一一对应。

// ---------------------------------------------------------------- 应用身份
pub const YSP_APPID: &str = "519748109";
pub const PLATFORM: &str = "5910204";
pub const APP_VER: &str = "V1.0.0";
pub const CHANNEL_TAG: &str = "ysp_tx";
pub const DEVICE_ID: &str = "mwpvxhkj_wotngaq1q1";

// ---------------------------------------------------------------- 签名密钥
pub const AUTH_KEY: &str = "n@7QKk%YeSjfw%22";
pub const LIVE_BODY_KEY: &str = "0f$IVHi9Qno?G";

// cKey：AES-128-CBC
pub const CKEY_AES_KEY: [u8; 16] = [
    0x48, 0xe5, 0x91, 0x8a, 0x74, 0xae, 0x21, 0xc9, 0x72, 0xb9, 0x0c, 0xce, 0x8a, 0xf6, 0xc8, 0xbe,
];
pub const CKEY_AES_IV: [u8; 16] = [
    0x9a, 0x7e, 0x7d, 0x23, 0x61, 0x02, 0x66, 0xb1, 0xd9, 0xfb, 0xf9, 0x85, 0x81, 0x38, 0x4d, 0x92,
];
pub const CKEY_PREFIX: &str = "--01";
pub const CKEY_SALT: &str = "mg3c3b04ba";

// yspticket：官方 CMG wasm 的 AES-CTR keystream（61 字节）
pub const TICKET_KEYSTREAM: [u8; 61] = [
    0x6a, 0x70, 0xe9, 0xac, 0x00, 0x7c, 0xcc, 0x38, 0x4c, 0x28, 0xa8, 0x8d, 0xfd, 0x22, 0x11, 0xde,
    0xcb, 0x66, 0x49, 0x4b, 0xca, 0x83, 0xbd, 0x61, 0x78, 0xf8, 0x57, 0x32, 0xb2, 0xea, 0xcc, 0x95,
    0x94, 0xe4, 0xe5, 0xab, 0x72, 0x0e, 0xc3, 0xa4, 0x3e, 0x87, 0xb3, 0xc7, 0x2b, 0x86, 0x19, 0x2f,
    0xf0, 0xdf, 0xea, 0x8f, 0xa8, 0xf4, 0x77, 0x94, 0xd8, 0x79, 0x26, 0x68, 0x43,
];

// ---------------------------------------------------------------- 接口地址
pub const PLAYER_API: &str = "https://player-api.yangshipin.cn";
pub const AUTH_URL: &str = "https://player-api.yangshipin.cn/v1/player/auth";
pub const LIVE_INFO_URL: &str = "https://player-api.yangshipin.cn/v1/player/get_live_info";

pub const H5ACCESS_URL: &str = "https://h5access.yangshipin.cn/web/open/token";
pub const H5ACCESS_VAPPID: &str = "59306155";
pub const H5ACCESS_VSECRET: &str = "b42702bf7309a179d102f3d51b1add2fda0bc7ada64cb801";
pub const H5ACCESS_VERSION: &str = "v1";

// yspsdksign 里的页面上下文
pub const SIGN_HOST: &str = "www.yangshipin.cn";
pub const SIGN_PROTOCOL: &str = "https:";

// 频道目录 / 节目单（protobuf）
pub const CHANNEL_PAGE_URL: &str = "https://capi.yangshipin.cn/api/oms/pc/page/PG00000004";
pub const EPG_URL_PREFIX: &str = "https://capi.yangshipin.cn/api/yspepg/program";

// ---------------------------------------------------------------- 终端形态
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/537.36 (KHTML, like Gecko) \
Chrome/152.0.7977.167 Electron/44.9.1 Safari/537.36";
pub const REFERER: &str = "https://www.yangshipin.cn/";
pub const ORIGIN: &str = "https://www.yangshipin.cn/";
pub const PAGE_URL: &str = "https://www.yangshipin.cn/tv/home";

// ---------------------------------------------------------------- 缓存时长
pub const TOKEN_TTL_SECS: u64 = 290;
pub const CATALOG_TTL_SECS: u64 = 12 * 3600;
pub const EPG_TTL_SECS: u64 = 300;
pub const PLAYURL_TTL_SECS: u64 = 4 * 3600 - 600;
pub const PLAYURL_REFRESH_SECS: u64 = 50 * 60;
/// 请求失败后的负缓存时间，避免反复打不可播频道。
pub const PLAYURL_ERROR_TTL_SECS: u64 = 120;

// ---------------------------------------------------------------- 分片
pub const TS_PACKET: usize = 188;
pub const DEFAULT_VIDEO_PID: u16 = 0x100;
/// 单个分片下载上限（正常 1080P 分片约 2 MB）。
pub const SEGMENT_MAX_BYTES: usize = 32 * 1024 * 1024;
/// 播放列表下载上限。
pub const PLAYLIST_MAX_BYTES: usize = 2 * 1024 * 1024;
/// 频道目录下载上限。
pub const PAGE_MAX_BYTES: usize = 8 * 1024 * 1024;
