#!/usr/bin/env bash
# 输出反汇编（从 .o 看真实编码）
# 用法: tools/objdump.sh <example 名 | .rs 文件> [opt-level]
#   RD_NO_DUMP=1  只生成 .o，不打印反汇编（供 evidence.sh 调用）
source "$(dirname "$0")/lib.sh"
rd_root
src=$(rd_resolve_src "${1:?usage: objdump.sh <example|file.rs> [opt]}")
opt="${2:-3}"
obj=$(rd_outdir)/"$(rd_prefix)$(basename "${src%.rs}")".O"$opt"$(rd_target_suffix).o

"$RD_RUSTC" --edition="$RD_EDITION" $(rd_target_arg) \
  $(rd_extern_args "$(rd_example_name "$src")") \
  -C opt-level="$opt" --emit "obj=$obj" --crate-type=lib "$src"

echo "OBJ    -> $obj" >&2
[[ -n "${RD_NO_DUMP:-}" ]] && exit 0

# 优先用 rustc 自带 sysroot 里的 llvm-objdump（版本与 rustc 的 LLVM 一致）；
# 找不到再退回 PATH（可能是 homebrew 的 llvm，版本可能不匹配）。
dump="$(find "$(rustc --print sysroot)/lib/rustlib" -name 'llvm-objdump' -type f 2>/dev/null | head -1 || true)"
[[ -z "$dump" ]] && dump="$(command -v llvm-objdump || true)"
if [[ -z "$dump" ]]; then
  echo "未找到 llvm-objdump（rustup component add llvm-tools）；已生成 $obj，可自行反汇编" >&2
  exit 0
fi
"$dump" -d --demangle --no-show-raw-insn "$obj"
