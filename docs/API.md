# HTTP API

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

## `/channels.m3u` 格式

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

## 分片与缓存

* `/live/{id}/{seg}.ts` 返回的是**解密后**的 TS，支持 `Range`，可直接拖动。
* 同一分片在任意并发下只下载、只解密一次，其余请求共享同一份 `Bytes`。
* 频道空闲 120 秒后缓存被回收；预算见
  [DEPLOY.md](DEPLOY.md#环境变量) 的 `YSP_SEGMENT_MEM_MB` / `YSP_CHANNEL_MEM_MB`。
