#!/usr/bin/env bash
# 第 8 章 · 两 crate 探针
#
# 为什么不能用 `rustc downstream.rs` 直接跑？
#   因为那样 upstream.rs 和 downstream.rs 是**同一个 crate**，
#   孤儿规则会直接放行（trait 是本地的）—— 证明不了任何东西。
#
# 这个脚本编译两次：
#   1) 把 upstream.rs 编译成 rlib（一个独立的 crate）
#   2) 用 --extern 把 downstream.rs 编译成**另一个** crate，链接上面的 rlib
#
# 只有第 2 步失败，才能证明"跨 crate 时孤儿规则生效"。
set -uo pipefail
cd "$(dirname "$0")"

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

echo "== 1) 编译 upstream（独立 crate）"
if ! rustc --edition 2024 --crate-type=lib --crate-name=upstream \
     upstream.rs -o "$out/libupstream.rlib" 2>&1; then
  echo "upstream 应当编译通过！"; exit 1
fi
echo "   OK -> libupstream.rlib"

echo
echo "== 2) 编译 downstream（另一个 crate，--extern 链接 upstream）"
if rustc --edition 2024 --crate-type=lib --crate-name=downstream \
     --extern "upstream=$out/libupstream.rlib" \
     downstream.rs -o "$out/libdownstream.rlib" 2>&1; then
  echo "downstream 竟然通过了 —— 孤儿规则没有生效！"; exit 1
fi

echo
echo "（上面的 E0210 就是预期结果：下游无法 blanket impl 上游的 trait）"
