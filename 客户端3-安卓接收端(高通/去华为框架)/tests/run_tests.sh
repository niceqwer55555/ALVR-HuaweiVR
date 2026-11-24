#!/usr/bin/env bash
# 沙箱可运行的核心逻辑单测
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=== C 核心单测 ==="
gcc -std=c11 -O2 -Wall \
  -I src/pose -I src/controller -I src/decoder -I src/runtime -I src/hid \
  tests/test_core.c \
  src/pose/pose.c src/controller/controller.c \
  src/decoder/decoder_policy.c src/runtime/generic_runtime.c src/hid/hwvr_hid.c \
  -lm -o /tmp/generic_alvr_test_core
/tmp/generic_alvr_test_core

echo ""
echo "=== Rust 客户端逻辑单测 ==="
( cd rustsrc && cargo test --quiet )
echo ""
echo "全部单测完成。"
