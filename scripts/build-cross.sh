#!/usr/bin/env bash
# 在 Debian/Ubuntu x86_64 上交叉编译 Linux(x86_64/aarch64) + Windows(x86_64)。
#
# 依赖：
#   apt-get install -y gcc-aarch64-linux-gnu gcc-mingw-w64-x86-64 musl-tools perl
#   rustup target add aarch64-unknown-linux-gnu x86_64-pc-windows-gnu x86_64-unknown-linux-musl
#
# 用法：bash scripts/build-cross.sh [输出目录，默认 dist]
set -euo pipefail

cd "$(dirname "$0")/.."
OUT="${1:-dist}"
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export CARGO_TERM_COLOR=never

mkdir -p "$OUT"

echo "== linux x86_64 (glibc)"
cargo build --release --locked --target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/ysp-server "$OUT/ysp-server-linux-x86_64"

echo "== linux x86_64 (musl, 静态)"
CC_x86_64_unknown_linux_musl=musl-gcc \
    cargo build --release --locked --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/ysp-server "$OUT/ysp-server-linux-x86_64-musl"

echo "== linux aarch64 (glibc)"
CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    cargo build --release --locked --target aarch64-unknown-linux-gnu
cp target/aarch64-unknown-linux-gnu/release/ysp-server "$OUT/ysp-server-linux-aarch64"

echo "== windows x86_64 (gnu)"
CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc \
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    cargo build --release --locked --target x86_64-pc-windows-gnu
cp target/x86_64-pc-windows-gnu/release/ysp-server.exe "$OUT/ysp-server-windows-x86_64.exe"

echo "== 完成"
ls -lh "$OUT"
