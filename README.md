# ysp-rs

央视频（CCTV / 地方卫视）直播平台，Python 版 `ysp-platform` 的 Rust 重构。
纯协议实现 + 本地 HLS 代理，**不需要浏览器、不需要 Node、不需要 wasm**。

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
├── deploy/ysp.service         # systemd 单元
└── deploy/deploy.sh           # 编译 + 安装 + 重启
```

依赖方向单向：`ysp-server -> ysp-net -> ysp-core`，
`ysp-server -> ysp-core`。核心层没有任何网络依赖。

## 针对 Python 版两个问题的处理

### 1. 多客户端同时播放卡顿

Python 版每个请求都独立 `threading` + 纯 Python 逐字节解密（一个 2 MB 分片约
1 秒），并且 `enc2` 路径没有「同分片只解一次」的合并，N 个客户端就是 N 次解密；
GIL 下还会互相抢。

Rust 版：

* **同分片单飞**：`OnceCell` 保证同一个分片在任意并发下只下载、只解密一次，
  其余请求等待并共享结果（`bytes::Bytes` 引用计数，天然零拷贝）。
* **解密不占 runtime**：全部丢进 `spawn_blocking`，用信号量限制并行度
  （`min(cores, 8)`），网络 IO 与 CPU 解密互不阻塞。
* **播放列表单飞**：每频道一把 `pl_fetch` 锁，1.5 秒内只打一次上游，
  且刷新期间不持有分片缓存锁。
* **全异步**：axum + hyper + reqwest 连接池，单线程可承载大量并发连接。
* 实测：一个 2 MB 分片解密从 Python 的 985 ms 降到 15 ms（见
  `ysp-server decrypt` 子命令）。

同一台机器（Debian 12 / 2 vCPU / 2 GB）上 20 个客户端同时拉流：

| 场景 | Python 版 | Rust 版 |
|---|---|---|
| 20 客户端请求同一新分片（冷） | **18.9 s**（p50） | **78 ms** |
| 20 客户端请求同一分片（缓存命中） | 10 ms | **1 ms** |
| 20 客户端混合请求 3 个分片 | **9.98 s**（p50） | **9 ms** |
| 进程 RSS（预热后 / 压测中） | 32 MB / 99 MB | 24 MB / 30 MB |

### 2. 内存占用高

Python 版缓存按「条数」计数（60 条 × 2 MB ≈ 120 MB/频道），
每个客户端还会各自持有解压后的 `bytes`；`_seg_urls` 无上限增长到 4000 条/频道。

Rust 版：

* 缓存**按字节预算**淘汰：全局默认 256 MB，单频道默认 32 MB，
  由 `MemBudget`（原子计数）统一记账，超预算先淘汰 LRU，再不行就不缓存。
* 分片全部用 `Bytes` 共享，同一分片无论多少客户端都只占一份内存。
* 空闲 120 秒的频道由 reaper 回收缓存。
* 播放列表只保留最新一份（约 1 KB），URL 表上限 512 条并按时效清理。
* `/api/stats` 直接暴露 `rss_mb`、缓存字节数、命中率，方便观察。
* 实测：9 个频道同时各拉一个分片，总缓存 25.8 MB，进程 RSS 48 MB。

## 构建 / 运行

```bash
# 环境：Rust stable（Linux / macOS 均可）
cargo build --release

./target/release/ysp-server serve --host 0.0.0.0 --port 8525 --warm
```

### 跨平台编译

同一份源码可产出 6 个平台的单文件可执行程序（TLS 用 rustls，不依赖系统 OpenSSL）：

| 目标 | 编译机 | 脚本 |
|---|---|---|
| `x86_64-unknown-linux-gnu` | Linux x86_64 原生 | `scripts/build-cross.sh` |
| `x86_64-unknown-linux-musl`（静态） | Linux x86_64 原生 | 同上 |
| `aarch64-unknown-linux-gnu` | Linux x86_64 交叉 | 同上 |
| `x86_64-pc-windows-gnu` | Linux x86_64 交叉 | 同上 |
| `aarch64-apple-darwin` | macOS 原生 | `scripts/build-macos.sh` |
| `x86_64-apple-darwin` | macOS 交叉 | 同上 |

```bash
# Debian/Ubuntu
apt-get install -y gcc-aarch64-linux-gnu gcc-mingw-w64-x86-64 musl-tools perl
rustup target add aarch64-unknown-linux-gnu x86_64-pc-windows-gnu x86_64-unknown-linux-musl
bash scripts/build-cross.sh dist

# macOS
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash scripts/build-macos.sh dist
```

其他命令（与 Python 版一一对应）：

```bash
ysp-server channels            # 非 VIP 频道
ysp-server channels --all      # 含 VIP
ysp-server playurl cctv13      # 打印 playurl JSON
ysp-server epg 湖南卫视        # 今日节目单
ysp-server probe --all --out probe.json
ysp-server decrypt enc2 in.ts out.ts   # 与 Python 版逐字节对比
```

环境变量：

| 变量 | 默认 | 说明 |
|---|---|---|
| `YSP_CACHE` | `.cache` | 目录/节目单缓存目录 |
| `YSP_GUID` | 内置 19 位 | 设备 ID（多实例请区分） |
| `YSP_PUBLIC_BASE` | 请求 Host | 反代/NAT 后对外地址，例如 `http://10.1.10.3:8525` |
| `YSP_DECRYPT` | 按响应 `encrypt` | 强制 `enc2` / `h5e` / `none` |
| `YSP_SEGMENT_MEM_MB` | 256 | 全局分片缓存预算 |
| `YSP_CHANNEL_MEM_MB` | 32 | 单频道分片缓存预算 |
| `RUST_LOG` | `info` | 日志级别 |

## HTTP API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/` | 频道目录页 |
| GET | `/play/{id}` | 播放页 |
| GET | `/api/channels` | 频道 JSON（`?all=1` 含 VIP） |
| GET | `/api/channels/{id}` | 频道详情 + 正在播出 |
| GET | `/api/channels/{id}/epg?date=YYYYMMDD` | 节目单 |
| GET | `/api/channels/{id}/playurl?defn=fhd` | 刷新播放地址 |
| GET | `/api/stats` | 进程 RSS / 缓存占用 / 命中率 |
| GET | `/live/{id}.m3u8` | 解密后的 media playlist |
| GET | `/live/{id}/master.m3u8` | 带 CODECS 的 master（浏览器用） |
| GET | `/live/{id}/{seg}.ts` | 解密分片（支持 Range） |
| GET | `/channels.m3u` | IPTV 频道清单 |
| GET | `/epg.xml` | XMLTV 节目单（`url-tvg` 指向它） |
| POST | `/admin/refresh[?channel=id]` | 手动刷新 |
| GET | `/healthz` | 健康检查 |

`{id}` 支持 `cctv13` 这种别名、`livepid`、中文名（URL 编码）。

### `/channels.m3u` 格式

```text
#EXTM3U url-tvg="http://10.1.10.3:8525/epg.xml"
#EXTINF:-1 tvg-id="cctv1" tvg-name="CCTV1" tvg-logo="https://.../cctv1.png" group-title="央视" ,CCTV1
http://10.1.10.3:8525/live/cctv1.m3u8
```

`tvg-id` 使用频道的稳定 ID（`/epg.xml` 里的 `<channel id>` 与之一致），
这样 IPTV 客户端能正确关联节目单；`tvg-name` / `tvg-logo` / `group-title`
按目录里的名称、台标、分类生成（央视 / 卫视 / 其他）。

> **Content-Type 别写成 `application/vnd.apple.mpegurl`**。
> 那是 HLS 媒体播放列表的 MIME，VLC 会把整份频道清单当成「一条直播流」交给
> adaptive/avformat 解复用，结果列表不展开。`/channels.m3u` 用
> `audio/x-mpegurl; charset=utf-8`（VLC 的 playlist stream_filter 认这个），
> `/live/{id}.m3u8` 才用 `application/vnd.apple.mpegurl`。

## 与 Python 版的一致性

* `cKey`、`yspticket`、auth/body/sdk 签名、protobuf 解析、m3u8 改写、
  enc2/h5e 解密均按原实现逐行移植，单测使用同一批真实抓包向量。
* 用真实分片做逐字节对比：

  ```bash
  # Python
  python3 -c "from ysp.media import decrypt; \
  open('plain.ts','wb').write(decrypt.decrypt('enc2', open('enc.ts','rb').read()).data)"
  # Rust
  ysp-server decrypt enc2 enc.ts plain-rs.ts
  sha256sum plain.ts plain-rs.ts
  ```

## 已知限制

* VIP 频道接口返回 `code=0` 但没有 `playurl`，目录里标记 `tier=vip`，默认不下发。
* 回看/点播协议（`cmd=3`）未实现；当前「点播」= 频道直播 + 节目单。
* `enc2` 解密输出为明文 RBSP（不再插入 `00 00 03`），这是正确行为，
  详见 Python 版 README 的说明。
