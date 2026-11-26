# 构建

## 环境

- Rust stable（`rustc 1.100` 实测）
- Linux / macOS 均可；Windows 建议直接用发布二进制
- TLS 使用 `rustls`（`ring`），**不需要系统 OpenSSL**

## 原生构建

```bash
cargo build --release
cargo test                       # 单元测试（crypto / protobuf / m3u8 / enc2 …）

./target/release/ysp-server serve --host 0.0.0.0 --port 8525 --warm
```

`[profile.release]` 默认 `opt-level=3`、`lto=false`、`codegen-units=16`、
`strip=true`，面向 2 GB 内存的小机器，`cargo build -j 2` 即可。

## 跨平台编译

同一份源码可产出 6 个平台的单文件可执行程序：

| 目标 | 编译机 | 脚本 |
|---|---|---|
| `x86_64-unknown-linux-gnu` | Linux x86_64 原生 | `scripts/build-cross.sh` |
| `x86_64-unknown-linux-musl`（静态） | Linux x86_64 原生 | 同上 |
| `aarch64-unknown-linux-gnu` | Linux x86_64 交叉 | 同上 |
| `x86_64-pc-windows-gnu` | Linux x86_64 交叉 | 同上 |
| `aarch64-apple-darwin` | macOS 原生 | `scripts/build-macos.sh` |
| `x86_64-apple-darwin` | macOS 交叉 | 同上 |

```bash
# Debian / Ubuntu
apt-get install -y gcc-aarch64-linux-gnu gcc-mingw-w64-x86-64 musl-tools perl
rustup target add aarch64-unknown-linux-gnu x86_64-pc-windows-gnu x86_64-unknown-linux-musl
bash scripts/build-cross.sh dist

# macOS（Xcode Command Line Tools + rustup）
rustup target add aarch64-apple-darwin x86_64-apple-darwin
bash scripts/build-macos.sh dist
```

产物命名与平台对应关系：

| 文件 | 平台 | 说明 |
|---|---|---|
| `ysp-server-linux-x86_64` | Linux x86_64 | glibc ≥ 2.34 |
| `ysp-server-linux-x86_64-musl` | Linux x86_64 | 全静态，任意发行版 |
| `ysp-server-linux-aarch64` | Linux arm64 | 树莓派 4/5、ARM 云主机 |
| `ysp-server-windows-x86_64.exe` | Windows x86_64 | Win10/11 |
| `ysp-server-macos-arm64` | macOS Apple Silicon | M1–M4 |
| `ysp-server-macos-x86_64` | macOS Intel | — |

## 与 Python 版逐字节对比

解密实现（`enc2` / `h5e`）与 Python 版 `ysp-platform` 逐行对齐，可用真实分片验证：

```bash
# Python
python3 -c "from ysp.media import decrypt; \
open('plain.ts','wb').write(decrypt.decrypt('enc2', open('enc.ts','rb').read()).data)"

# Rust
ysp-server decrypt enc2 enc.ts plain-rs.ts
sha256sum plain.ts plain-rs.ts
```

## 校验发布产物

```bash
shasum -a 256 -c SHA256SUMS        # macOS / Linux
certutil -hashfile <file> SHA256   # Windows
```
