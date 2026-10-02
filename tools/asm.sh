#!/usr/bin/env bash
# 输出汇编（看契约在硬件上的兑现：内联、向量化、原子指令、panic 路径）
# 用法: tools/asm.sh <example 名 | .rs 文件> [opt-level]
set -euo pipefail
source "$(dirname "$0")/lib.sh"
rd_root
src=$(rd_resolve_src "${1:?usage: asm.sh <example|file.rs> [opt]}")
opt="${2:-3}"
out=$(rd_outdir)/"$(rd_prefix)$(basename "${src%.rs}")".O"$opt"$(rd_target_suffix).s
rd_prepare_rustc_args "$(rd_example_name "$src")"

"$RD_RUSTC" "${RD_RUSTC_ARGS[@]}" -C opt-level="$opt" \
  --emit "asm=$out" --crate-type=lib "$src"

echo "ASM    -> $out"
