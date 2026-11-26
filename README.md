# ysp-rs

央视频（CCTV / 地方卫视）直播平台，Python 版 `ysp-platform` 的 Rust 重构。

本地跑一个单文件服务，把央视频 **1080P（`encrypt=2`）** 直播流解密成标准 HLS，
浏览器 / VLC / IPTV 客户端直接播放。纯协议实现，**不需要浏览器、不需要 Node、
不需要 wasm**。

## 特性

* **1080P 优先**：`encrypt=2` 直出 1920×1080，失败自动回落 720P（`h5e`）/ 540P
* **全频道**：CCTV / CGTN 40 个 + 地方卫视 24 个，目录按官方页面配置动态解析
* **开箱即用**：目录页、播放页、EPG、`/channels.m3u`、`/epg.xml` 全都内置
* **单文件**：六个平台可执行程序（Linux x86_64/arm64、Windows、macOS Intel/ARM）
* **省资源**：同分片只下载/解密一次，多客户端共享；内存按字节预算淘汰

## 快速开始

从源码构建（Rust stable）：

```bash
cargo build --release
./target/release/ysp-server serve --host 0.0.0.0 --port 8525 --warm
```

打开 <http://127.0.0.1:8525/> 即可。

也可以直接使用发布二进制，解压后同样执行 `ysp-server serve ...`；
编译方法见 [docs/BUILD.md](docs/BUILD.md)。

## 使用方法

### 网页

| 地址 | 内容 |
|---|---|
| `/` | 频道目录（分组 + 搜索） |
| `/play/cctv13` | 播放页（含「正在播出」） |

### 直接播放

| 地址 | 用途 |
|---|---|
| `/live/cctv13.m3u8` | 标准 HLS，VLC / IINA / ffmpeg 均可 |
| `/live/cctv13/master.m3u8` | 带 `CODECS` 的 master（浏览器 MSE 用） |
| `/live/600002475.m3u8` | 卫视用数字 ID（湖南卫视） |
| `/channels.m3u` | IPTV 客户端频道清单（自动带 `url-tvg`） |
| `/epg.xml` | XMLTV 节目单 |

```bash
ffplay http://127.0.0.1:8525/live/cctv13.m3u8
vlc    http://127.0.0.1:8525/channels.m3u
```

### 命令行

```bash
ysp-server serve               # 启动服务（--host / --port / --warm）
ysp-server channels            # 非 VIP 频道
ysp-server channels --all      # 含 VIP
ysp-server playurl cctv13      # 打印 playurl JSON
ysp-server epg 湖南卫视        # 今日节目单
ysp-server probe --all --out probe.json
ysp-server decrypt enc2 in.ts out.ts   # 单独解密一个分片
```

### 常用环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `YSP_CACHE` | `.cache` | 目录/节目单缓存目录 |
| `YSP_PUBLIC_BASE` | 请求 Host | 反代/NAT 后对外地址 |
| `YSP_DECRYPT` | 按响应 `encrypt` | 强制 `enc2` / `h5e` / `none` |
| `YSP_SEGMENT_MEM_MB` | 256 | 全局分片缓存预算 |
| `YSP_CHANNEL_MEM_MB` | 32 | 单频道分片缓存预算 |
| `RUST_LOG` | `info` | 日志级别 |

完整列表和部署方式见 [docs/DEPLOY.md](docs/DEPLOY.md)。

## 文档

| 文档 | 内容 |
|---|---|
| [docs/BUILD.md](docs/BUILD.md) | 构建、六平台交叉编译、产物校验 |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 目录结构、分层依赖、数据流 |
| [docs/DESIGN.md](docs/DESIGN.md) | 并发/内存设计与 Python 版压测对比 |
| [docs/API.md](docs/API.md) | HTTP API、`/channels.m3u` 格式 |
| [docs/DEPLOY.md](docs/DEPLOY.md) | systemd、反代/NAT、全部环境变量 |

## 已知限制

* VIP 频道接口返回 `code=0` 但没有 `playurl`，目录里标记 `tier=vip`，默认不下发。
* 回看/点播协议（`cmd=3`）未实现；当前「点播」= 频道直播 + 节目单。
* 单个频道偶发上游不可播（平台侧临时限制），服务会自动标记并跳过。

> 仅用于个人技术研究与本地观看，内容版权归权利人所有，请遵守平台条款。
