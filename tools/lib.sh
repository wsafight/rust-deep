#!/usr/bin/env bash
# 公共函数：供 mir.sh / llvm.sh / asm.sh / objdump.sh 复用
set -euo pipefail

# 统一口径（书中的版本标注以此为准）
export RD_EDITION="${RD_EDITION:-2024}"
export RD_TARGET="${RD_TARGET:-}"
export RD_RUSTC="${RD_RUSTC:-rustc}"

rd_root() { cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd; }

# 解析输入：既接受 .rs 文件，也接受 example 名（ch01-borrow）
rd_resolve_src() {
  local arg="$1"
  if [[ -d "examples/$arg" ]]; then
    local f="examples/$arg/src/lib.rs"
    [[ -f "$f" ]] || f="examples/$arg/src/main.rs"
    echo "$f"
  else
    echo "$arg"
  fi
}

rd_outdir() { mkdir -p .evidence; echo ".evidence"; }

# 证据文件名前缀（由 evidence.sh 设为 example 名，避免各章都叫 lib.*）
rd_prefix() { echo "${RD_PREFIX:-}"; }

rd_target_arg() {
  [[ -n "$RD_TARGET" ]] && printf -- '--target=%s' "$RD_TARGET" || true
}

# 目标架构后缀：未指定 target 时为空（宿主架构）；指定时用于文件名隔离
#   x86_64-apple-darwin  ->  .x86_64
rd_target_suffix() {
  [[ -z "$RD_TARGET" ]] && return 0
  echo ".${RD_TARGET%%-*}"
}

# 从已解析的源文件路径反推 example 名
#   examples/ch22-tokio/src/lib.rs  ->  ch22-tokio
#   其他路径（如 /tmp/x.rs）        ->  目录名（通常没有 externs，返回空）
rd_example_name() {
  local src="$1"
  case "$src" in
    examples/*/*/*) basename "$(dirname "$(dirname "$src")")" ;;
    *)              basename "$(dirname "$src")" ;;
  esac
}

# ★ 该 example 依赖的外部 crate 的 --extern 参数（可能为空）
#
# 为什么需要：本书大部分 example 是**零依赖**的，所以直接 `rustc x.rs` 就行。
# 但少数章（如 ch22-tokio）必须链接真实的第三方 crate。
# 那些章在 `examples/<name>/externs` 里逐行写 crate 名，
# 这里用 `cargo build --message-format=json` 找出对应的 .rlib 路径。
rd_extern_args() {
  local name="$1"
  local f="examples/$name/externs"
  [[ -f "$f" ]] || return 0
  # 先确保依赖已构建（否则找不到 .rlib）
  cargo build --locked -q -p "$name" 2>/dev/null || true
  local json
  json=$(cargo build --locked -q -p "$name" --message-format=json 2>/dev/null) || return 0
  # ★ 传递依赖也要给：rustc 直接调时需要能解析 tokio 的全部依赖
  #   （cargo 会自动做这件事，裸 rustc 不会）
  local -a wanted=()
  while read -r crate; do
    [[ -z "$crate" || "$crate" == \#* ]] && continue
    wanted+=("$crate")
  done < "$f"

  local crate out
  for crate in "${wanted[@]}"; do
    [[ -z "$crate" || "$crate" == \#* ]] && continue
    # 从 JSON 里找 "name":"<crate>" 且 filenames 以 .rlib 结尾的 artifact
    out=$(printf '%s\n' "$json" | python3 -c '
import json,sys
want=sys.argv[1]
for line in sys.stdin:
    line=line.strip()
    if not line.startswith("{"): continue
    try: m=json.loads(line)
    except Exception: continue
    if m.get("reason")!="compiler-artifact": continue
    t=m.get("target") or {}
    if t.get("name")!=want: continue
    for fn in (m.get("filenames") or []):
        if fn.endswith(".rlib"):
            print(fn); break
' "$crate")
    [[ -n "$out" ]] && printf -- '--extern=%s=%s ' "$crate" "$out"
  done

  # ★ 关键：把依赖目录加进搜索路径。
  #   只给 `--extern=tokio=...` 是不够的 —— rustc 从 tokio 的 metadata 里
  #   读到它依赖 pin_project_lite 等 crate，然后**按 -L 的搜索路径去找**。
  #   （报错形态：E0463 "can't find crate for `X` which `tokio` depends on"）
  local deps
  deps=$(printf '%s\n' "$json" | python3 -c '
import json,sys,os
dirs=set()
for line in sys.stdin:
    line=line.strip()
    if not line.startswith("{"): continue
    try: m=json.loads(line)
    except Exception: continue
    if m.get("reason")!="compiler-artifact": continue
    for fn in (m.get("filenames") or []):
        if fn.endswith(".rlib"):
            dirs.add(os.path.dirname(fn))
            break
for d in sorted(dirs):
    print(d)
')
  local d
  while read -r d; do
    [[ -n "$d" ]] && printf -- '-L dependency=%s ' "$d"
  done <<< "$deps"
}
