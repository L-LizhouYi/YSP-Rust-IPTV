# 部署

## 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `YSP_CACHE` | `.cache` | 目录/节目单缓存目录 |
| `YSP_GUID` | 内置 19 位 | 设备 ID（多实例请区分） |
| `YSP_PUBLIC_BASE` | 请求 Host | 反代/NAT 后对外地址，例如 `http://10.1.10.3:8525` |
| `YSP_DECRYPT` | 按响应 `encrypt` | 强制 `enc2` / `h5e` / `none` |
| `YSP_SEGMENT_MEM_MB` | 256 | 全局分片缓存预算 |
| `YSP_CHANNEL_MEM_MB` | 32 | 单频道分片缓存预算 |
| `RUST_LOG` | `info` | 日志级别 |

## systemd（Debian / Ubuntu）

`deploy/ysp.service` 已包含 2 GB 小机器的内存上限（`MemoryHigh=600M`、
`MemoryMax=1000M`）。一键部署：

```bash
bash deploy/deploy.sh
```

脚本会 `cargo build --release`、安装到 `/usr/local/bin/ysp-server`、
写入 `/etc/systemd/system/ysp.service`、`systemctl enable --now`。

手动方式：

```bash
install -m 0755 target/release/ysp-server /usr/local/bin/ysp-server
install -d /var/lib/ysp
install -m 0644 deploy/ysp.service /etc/systemd/system/ysp.service
systemctl daemon-reload && systemctl enable --now ysp.service
journalctl -u ysp -f
```

## 反向代理 / NAT

放在 Nginx、Caddy 或路由器端口映射后面时，设置
`YSP_PUBLIC_BASE`，否则 `/channels.m3u` 和播放列表里的地址会是内网 Host：

```bash
YSP_PUBLIC_BASE=http://10.1.10.3:8525 ysp-server serve --host 0.0.0.0 --port 8525 --warm
```

Nginx 示例（HLS 分片无需缓冲）：

```nginx
location / {
    proxy_pass http://127.0.0.1:8525;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_buffering off;
}
```

## 端口与并发

* 默认监听 `127.0.0.1:8525`，要局域网访问用 `--host 0.0.0.0`。
* `LimitNOFILE=65535` 已写在 unit 里；单进程异步即可承载大量并发连接。
* 解密并发上限为 `min(CPU 核数, 8)`，2 核机器不要同时开太多频道预热。

## 观察

```bash
curl -s http://127.0.0.1:8525/api/stats | python3 -m json.tool
curl -s http://127.0.0.1:8525/healthz
```

`/api/stats` 给出 `rss_mb`、缓存字节数与命中率，用来判断内存预算是否合适。
