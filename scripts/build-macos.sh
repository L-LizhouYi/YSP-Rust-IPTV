#!/usr/bin/env bash
# 在 Apple Silicon Mac 上编译 macOS(arm64/x86_64)。Intel Mac 同样可用（arm64 为交叉）。
#
# 依赖：Xcode Command Line Tools + rustup
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
#
# 用法：bash scripts/build-macos.sh [输出目录，默认 dist]
set -euo pipefail

cd "$(dirname "$0")/.."
OUT="${1:-dist}"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}"
export CARGO_TERM_COLOR=never

mkdir -p "$OUT"

echo "== macos arm64"
cargo build --release --locked --target aarch64-apple-darwin
cp target/aarch64-apple-darwin/release/ysp-server "$OUT/ysp-server-macos-arm64"

echo "== macos x86_64"
cargo build --release --locked --target x86_64-apple-darwin
cp target/x86_64-apple-darwin/release/ysp-server "$OUT/ysp-server-macos-x86_64"

echo "== 完成"
ls -lh "$OUT"
