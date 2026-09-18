#!/usr/bin/env bash
# 输出 MIR（借用检查之后的产物）
# 用法: tools/mir.sh <example 名 | .rs 文件>
source "$(dirname "$0")/lib.sh"
rd_root
src=$(rd_resolve_src "${1:?usage: mir.sh <example|file.rs>}")
out=$(rd_outdir)/"$(rd_prefix)$(basename "${src%.rs}")"$(rd_target_suffix).mir

# 注意：必须用 -o 指定正常文件名；用 /dev/null 会报 could not create a temp dir
"$RD_RUSTC" --edition="$RD_EDITION" $(rd_target_arg) $(rd_extern_args "$(rd_example_name "$src")") \
  --emit=mir -o "$out" --crate-type=lib "$src"

echo "MIR  -> $out"
