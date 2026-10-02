#!/usr/bin/env bash
# 输出 LLVM IR（看 noalias / readonly / nonnull 等契约元数据）
# 用法: tools/llvm.sh <example 名 | .rs 文件> [opt-level]
set -euo pipefail
source "$(dirname "$0")/lib.sh"
rd_root
src=$(rd_resolve_src "${1:?usage: llvm.sh <example|file.rs> [opt]}")
opt="${2:-3}"
out=$(rd_outdir)/"$(rd_prefix)$(basename "${src%.rs}")".O"$opt"$(rd_target_suffix).ll
rd_prepare_rustc_args "$(rd_example_name "$src")"

"$RD_RUSTC" "${RD_RUSTC_ARGS[@]}" -C opt-level="$opt" \
  --emit "llvm-ir=$out" --crate-type=lib "$src"

echo "LLVM IR -> $out"
