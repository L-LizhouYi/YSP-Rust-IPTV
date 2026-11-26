#!/usr/bin/env bash
# 在目标机上编译 + 安装 + 重启（Debian 12 / 2G 内存默认 -j2）。
set -euo pipefail

cd "$(dirname "$0")/.."
SRC_DIR="$(pwd)"

export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"

echo "== build =="
cargo build --release --locked 2>/dev/null || cargo build --release

echo "== install =="
install -d /opt/ysp-rs
install -m 0755 target/release/ysp-server /usr/local/bin/ysp-server
install -d /var/lib/ysp
install -m 0644 "$SRC_DIR/deploy/ysp.service" /etc/systemd/system/ysp.service

echo "== systemd =="
systemctl daemon-reload
systemctl enable ysp.service
systemctl restart ysp.service
sleep 1
systemctl --no-pager --lines=20 status ysp.service || true
