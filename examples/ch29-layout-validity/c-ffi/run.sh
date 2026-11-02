#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../../.."
temporary=$(mktemp -d "${TMPDIR:-/tmp}/rust-deep-ffi.XXXXXX")
trap 'rm -rf -- "$temporary"' EXIT
rustc --edition=2024 --crate-type=staticlib -C panic=abort \
  examples/ch29-layout-validity/src/ffi.rs -o "$temporary/libchecked_add.a"
libraries=()
case "$(uname -s)" in
  Linux) libraries=(-ldl -lpthread -lm) ;;
  Darwin) ;;
  *) echo "C ABI probe supports native Linux and macOS hosts" >&2; exit 1 ;;
esac
"${CC:-cc}" -std=c11 -Wall -Wextra -Werror \
  examples/ch29-layout-validity/c-ffi/main.c "$temporary/libchecked_add.a" \
  ${libraries[@]+"${libraries[@]}"} -o "$temporary/check"
"$temporary/check"
printf '%s\n' 'PASS: C calls Rust with matching layout, symbol and calling convention'
