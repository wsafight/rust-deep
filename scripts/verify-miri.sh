#!/usr/bin/env bash
# Miri 证据验证（**与 verify-all.sh 分开**）
#
# 为什么分开：Miri 是独立的 MIR 解释器，只随 **nightly** 发布，
# 而书的其余部分坚持 stable。而且它的用例语义与常规断言相反：
#   - `tests/sb_legal.rs`  必须在 Miri 下**通过**（不误报）
#   - `tests/sb_ub.rs`     必须在 Miri 下**失败**（真的抓到 UB）
# 后者在普通 `cargo test` 下会"通过"（UB 不一定当场崩），
# 所以不能混进 verify-all.sh 的常规流程。
#
# 用法: scripts/verify-miri.sh
set -uo pipefail
cd "$(dirname "$0")/.."

pass=0
fail=0
ok()  { printf '  \033[32mPASS\033[0m %s\n' "$1"; pass=$((pass+1)); }
bad() { printf '  \033[31mFAIL\033[0m %s\n' "$1"; fail=$((fail+1)); }

printf '\n\033[1m== Miri 前置检查 ==\033[0m\n'
if ! cargo +nightly miri --version >/dev/null 2>&1; then
  bad "cargo +nightly miri 不可用"
  echo "      装：rustup toolchain install nightly -c rustc -c miri -c rust-src"
  echo "          cargo +nightly miri setup"
  printf '\n  PASS=%d  FAIL=%d\n' "$pass" "$fail"
  exit 1
fi
ok "cargo +nightly miri 可用（$(cargo +nightly miri --version)）"

# 通用：跑一个 Miri 测试目标，断言"必须通过"或"必须报 UB"
#   miri_must_pass <crate> <test 名> <说明>
#   miri_must_ub   <crate> <test 名> <说明>
# ⚠️ 变量引用一律用 ${var} 花括号形式：macOS 自带的是 bash 3.2，
#    它会把紧跟其后的多字节字符（如 `（`）当成变量名的一部分，
#    在 `set -u` 下直接报 unbound variable。
miri_must_pass() {
  local pkg="$1" t="$2" desc="$3" out="/tmp/rd-miri-${1}-${2}"
  if cargo +nightly miri test -q -p "$pkg" --test "$t" >"$out" 2>&1; then
    ok "${pkg}/${t} 在 Miri 下通过（${desc}）"
  else
    bad "${pkg}/${t} 在 Miri 下失败了（${desc}）—— 要么 Miri 误报，要么代码其实不合法"
    sed 's/^/      /' "$out" | tail -20
  fi
}

miri_must_ub() {
  local pkg="$1" t="$2" desc="$3" out="/tmp/rd-miri-${1}-${2}"
  if cargo +nightly miri test -q -p "$pkg" --test "$t" >"$out" 2>&1; then
    bad "${pkg}/${t} 在 Miri 下**通过了** —— 期望它报 UB（${desc}）"
  else
    # 必须是"报出 Undefined Behavior"，而不是别的错误（比如编译失败）
    if grep -q 'Undefined Behavior' "$out"; then
      ok "${pkg}/${t} 在 Miri 下报出 Undefined Behavior（${desc}）"
    else
      bad "${pkg}/${t} 失败了，但不是 UB 错误（${desc}）—— 可能是别的问题"
      sed 's/^/      /' "$out" | tail -20
    fi
  fi
}

printf '\n\033[1m== ch25：合法代码不得误报 / UB 代码必须抓到 ==\033[0m\n'
miri_must_pass ch25-aliasing sb_legal   "手工裸指针下的合法边界"
miri_must_ub   ch25-aliasing sb_ub      "失效的共享引用 + 悬垂指针"
miri_must_pass ch25-aliasing aliasing   "★ UnsafeCell / PhantomData 的合法用法"
miri_must_ub   ch25-aliasing aliasing_ub "★ 顺序反了：先建 &T 再写、或跨函数藏起铸型点"

printf '\n\033[1m== ch19：自引用结构（Pin 那一章）==\033[0m\n'
miri_must_pass ch19-pin selfref    "addr_of_mut! 派生的自引用指针 sound"
miri_must_ub   ch19-pin selfref_ub "addr_of! 派生（写后失效）+ 移动后悬垂"

printf '\n\033[1m== ch23：mini async runtime（手写 Waker）==\033[0m\n'
# ★ 这是本书里 Miri 最"物有所值"的一次：
#   手写 RawWakerVTable 的四个函数必须配平引用计数 ——
#   错了就是内存泄漏或 use-after-free，而**编译器完全看不见**。
miri_must_pass ch23-project-runtime scheduler "★ 手写 waker 的引用计数配平（含跨线程唤醒）"

printf '\n\033[1m== 结果 ==\033[0m\n'
printf '  PASS=%d  FAIL=%d\n' "$pass" "$fail"
[[ "$fail" -eq 0 ]] || exit 1
