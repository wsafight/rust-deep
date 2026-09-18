#!/usr/bin/env bash
# 一次性生成某章的全部证据（MIR + LLVM IR + 汇编 O0/O3 + .o）
# 用法: tools/evidence.sh <example 名> [target]
#   target 省略时为宿主架构；给 x86_64-apple-darwin 可出对照证据
#   （文件名会带 .x86_64 后缀，不会覆盖宿主架构的证据）
set -euo pipefail
source "$(dirname "$0")/lib.sh"
rd_root
name="${1:?usage: evidence.sh <example> [target]}"
[[ -n "${2:-}" ]] && export RD_TARGET="$2"
# 证据文件名以 example 名为前缀，避免不同章都叫 lib.*
export RD_PREFIX="$name-"
for opt in 0 3; do
  tools/llvm.sh "$name" "$opt"
  tools/asm.sh  "$name" "$opt"
  # 反汇编证据（.o 落盘；反汇编文本只在需要时手动跑 tools/objdump.sh 看）
  RD_NO_DUMP=1 tools/objdump.sh "$name" "$opt"
done
tools/mir.sh "$name"
echo "--- 证据文件 ---"
ls -la .evidence/ | tail -n +2
