#!/usr/bin/env bash
# 一次性生成某章的文本证据（MIR + LLVM IR + 汇编 O0/O3）
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
src=$(rd_resolve_src "$name")
base=$(rd_outdir)/"$(rd_prefix)$(basename "${src%.rs}")"
suffix=$(rd_target_suffix)
rd_prepare_rustc_args "$name"
for opt in 0 3; do
  ll="${base}.O${opt}${suffix}.ll"
  asm="${base}.O${opt}${suffix}.s"
  "$RD_RUSTC" "${RD_RUSTC_ARGS[@]}" -C opt-level="$opt" \
    --emit "llvm-ir=$ll,asm=$asm" --crate-type=lib "$src"
  echo "LLVM IR -> $ll"
  echo "ASM    -> $asm"
done
mir="${base}${suffix}.mir"
"$RD_RUSTC" "${RD_RUSTC_ARGS[@]}" --emit=mir -o "$mir" --crate-type=lib "$src"
echo "MIR    -> $mir"
echo "--- 证据文件 ---"
ls -la .evidence/ | tail -n +2
