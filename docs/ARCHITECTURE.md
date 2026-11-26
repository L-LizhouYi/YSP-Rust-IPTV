# 架构

## 目录结构

```
ysp-rs/
├── Cargo.toml                 # workspace
├── crates/
│   ├── ysp-core/              # 纯算法（无 IO，可单测）
│   │   ├── config.rs          # 全部常量/密钥
│   │   ├── models.rs          # Channel / Program / PlayInfo
│   │   ├── crypto/            # cKey / yspticket / 各类签名
│   │   ├── protocol/          # protobuf / 频道目录 / 节目单
│   │   └── media/             # m3u8、TS、enc2(1080P)、h5e(720P)
│   ├── ysp-net/               # 上游接口（有 IO）
│   │   ├── http.rs            # UA/Referer/重试/响应大小上限
│   │   ├── token.rs           # h5access token（单飞 + 缓存）
│   │   ├── signer.rs          # yspsdksign（纯 md5）
│   │   ├── player.rs          # auth + get_live_info
│   │   ├── catalog.rs         # 频道目录（磁盘缓存）
│   │   └── epg.rs             # 节目单（内存 LRU + 磁盘缓存）
│   └── ysp-server/            # axum 服务
│       ├── app.rs             # 装配 + 目录服务
│       ├── playurl.rs         # 每频道一把锁，刷新单飞
│       ├── proxy.rs           # HLS 代理、分片缓存、解密并发控制
│       ├── routes.rs          # 路由 / Range / CORS
│       ├── epgxml.rs          # /epg.xml（XMLTV）
│       ├── m3u.rs             # /channels.m3u
│       └── web.rs             # 目录页 / 播放页
├── deploy/
│   ├── ysp.service            # systemd 单元
│   └── deploy.sh              # 编译 + 安装 + 重启
└── scripts/
    ├── build-cross.sh         # Linux ×3 + Windows
    └── build-macos.sh         # macOS ×2
```

## 依赖方向

```
ysp-server ──> ysp-net ──> ysp-core
     └──────────────────> ysp-core
```

严格单向，`ysp-core` 不依赖任何网络/异步运行时，可直接单测；`ysp-net`
只做上游协议与 IO；`ysp-server` 只做 HTTP、缓存与进程装配。

## 各层职责

| crate | 职责 | 可测试性 |
|---|---|---|
| `ysp-core` | 常量、模型、加密/签名、protobuf、m3u8、TS 解封装与解密 | 纯函数，`cargo test` 覆盖 |
| `ysp-net` | 央视频接口：auth、get_live_info、h5access token、频道目录、EPG | mock HTTP |
| `ysp-server` | axum 路由、HLS 代理、分片缓存、解密调度、Web 页面 | 路由/清单单测 |

## 数据流

```
浏览器 / VLC
   │  GET /live/cctv13.m3u8
   ▼
ysp-server ── playurl 缓存 ──> ysp-net ── auth + get_live_info ──> 央视频
   │                              └── h5access token（5 分钟缓存）
   │  GET /live/cctv13/{seg}.ts
   ▼
proxy.rs ── 分片缓存命中？ ── 否 ──> 下载上游分片
   │                                   │
   │                                   ▼
   └────────────── spawn_blocking ── enc2 / h5e 解密 ──> 明文 TS
```

同一分片在任意并发下只下载、只解密一次，其余请求等待并共享 `bytes::Bytes`。
