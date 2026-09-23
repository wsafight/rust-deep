#!/usr/bin/env bash
# 全书证据验证：编译所有 example + 运行所有断言
# 用法: scripts/verify-all.sh        （全部）
#       scripts/verify-all.sh ch07   （只跑匹配的章）
set -uo pipefail
cd "$(dirname "$0")/.."

filter="${1:-}"
fail=0
pass=0
skipped=0

section() { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }
ok()      { printf '  \033[32mPASS\033[0m %s\n' "$1"; pass=$((pass+1)); }
bad()     { printf '  \033[31mFAIL\033[0m %s\n' "$1"; fail=$((fail+1)); }
skip()    { printf '  \033[33mSKIP\033[0m %s\n' "$1"; skipped=$((skipped+1)); }
target_is_installed() { rustup target list --installed 2>/dev/null | grep -qx "$1"; }

# ---------- 1) 所有 example 必须能编译 ----------
section "编译 examples"
for dir in examples/*/; do
  name=$(basename "$dir")
  [[ -n "$filter" && "$name" != *"$filter"* ]] && continue
  if cargo build --locked -q -p "$name" 2>/tmp/rd-build-err; then
    ok "cargo build -p $name"
  else
    bad "cargo build -p ${name}"
    sed 's/^/      /' /tmp/rd-build-err | head -20
  fi
done

# ---------- 2) 生成证据 ----------
section "生成证据 (.evidence/)"
for dir in examples/*/; do
  name=$(basename "$dir")
  [[ -n "$filter" && "$name" != *"$filter"* ]] && continue
  if tools/evidence.sh "$name" >/tmp/rd-ev-err 2>&1; then
    ok "evidence.sh $name"
  else
    bad "evidence.sh ${name}"
    sed 's/^/      /' /tmp/rd-ev-err | head -20
  fi
  # 额外 target 的对照证据（每行一个 target，见 examples/*/cross-targets）
  if [[ -f "$dir/cross-targets" ]]; then
    while read -r tgt; do
      [[ -z "$tgt" || "$tgt" == \#* ]] && continue
      if ! target_is_installed "$tgt"; then
        skip "evidence.sh $name ($tgt 未安装，可按附录 A 安装)"
      elif tools/evidence.sh "$name" "$tgt" >/tmp/rd-ev-err 2>&1; then
        ok "evidence.sh $name ($tgt)"
      else
        bad "evidence.sh ${name} (${tgt})"
        sed 's/^/      /' /tmp/rd-ev-err | head -20
      fi
    done < "$dir/cross-targets"
  fi
done

# ---------- 3) 断言 ----------
# 格式: assert_contains <example> <文件名后缀> <正则> <说明>
# 这些断言就是书中结论的"活体检查"：编译器一升级、结论过期就会亮红灯。
# 注意：断言也受 $filter 过滤（见 assert_hit 首行）。
assert_hit() {
  local mode="$1" ex="$2" file="$3" pat="$4" desc="$5"
  [[ -n "$filter" && "$ex" != *"$filter"* ]] && return
  local f
  # ⚠️ 必须用**完整** example 名做前缀：早期版本写成 ${ex%%-*}，
  # 于是 ch05-project-tree 会匹配到 ch05-bounds 的证据（head -1 取第一个）。
  f=$(ls .evidence/"${ex}"-*"${file}" 2>/dev/null | head -1)
  if [[ -z "$f" ]]; then bad "${ex}: missing evidence *${file}"; return; fi
  if grep -qE "$pat" "$f"; then
    if [[ "$mode" == contains ]]; then ok "$ex: $desc"; else bad "${ex}: ${desc} (unexpected /${pat}/)"; fi
  else
    if [[ "$mode" == contains ]]; then bad "${ex}: ${desc} (no match /${pat}/, see ${f})"; else ok "$ex: $desc"; fi
  fi
}
assert_contains()     { assert_hit contains     "$@"; }
assert_not_contains() { assert_hit not_contains "$@"; }

# 跨行断言：pat1 的下一行匹配 pat2
# 格式: assert_adjacent <example> <文件名后缀> <第一行正则> <第二行正则> <说明>
assert_adjacent() {
  local ex="$1" file="$2" pat1="$3" pat2="$4" desc="$5"
  [[ -n "$filter" && "$ex" != *"$filter"* ]] && return
  local f
  f=$(ls .evidence/"${ex}"-*"${file}" 2>/dev/null | head -1)
  if [[ -z "$f" ]]; then bad "${ex}: missing evidence *${file}"; return; fi
  if grep -A1 -E "$pat1" "$f" | grep -qE "$pat2"; then ok "$ex: $desc"
  else bad "${ex}: ${desc} (no adjacent /${pat1}/ + /${pat2}/, see ${f})"; fi
}

# 函数作用域断言：只看某个符号的**函数体**里有没有匹配
# 格式: assert_fn_contains <example> <文件名后缀> <符号> <正则> <说明>
#       assert_fn_not_contains 同理
# 为什么需要它：`.O3.s` 里几百个符号，`blr`/`ldr` 到处都是 ——
# 全文件 grep 说明不了"**这个函数**里没有间接跳转"。
# awk 的区间是 `^符号:` 到 `cfi_endproc`（rustc 每个函数都以它结尾）。
assert_fn_hit() {
  local mode="$1" ex="$2" file="$3" sym="$4" pat="$5" desc="$6"
  [[ -n "$filter" && "$ex" != *"$filter"* ]] && return
  local f body
  f=$(ls .evidence/"${ex}"-*"${file}" 2>/dev/null | head -1)
  if [[ -z "$f" ]]; then bad "${ex}: missing evidence *${file}"; return; fi
  # ⚠️ 符号可能带前导下划线（macOS 的 Mach-O 约定），
  #    所以匹配 `^_?符号:`；`?` 让调用方既能传 `use_dyn` 也能传 `_use_dyn`。
  body=$(awk -v s="^_?${sym}:" '$0 ~ s {on=1} on {print} on && /cfi_endproc/ {exit}' "$f")
  # ⚠️ 变量引用一律用 ${var}：macOS bash 3.2 会把紧跟的多字节字符（如 `（`）
  #    当成变量名的一部分，在 set -u 下直接报 unbound variable。
  if [[ -z "$body" ]]; then bad "$ex: 找不到符号 ${sym}（见 ${f}）"; return; fi
  if grep -qE "$pat" <<<"$body"; then
    if [[ "$mode" == contains ]]; then ok "$ex: $desc"; else bad "$ex: $desc (unexpected /$pat/ in ${sym})"; fi
  else
    if [[ "$mode" == contains ]]; then bad "$ex: $desc (no /$pat/ in ${sym}, see $f)"; else ok "$ex: $desc"; fi
  fi
}
assert_fn_contains()     { assert_fn_hit contains     "$@"; }
assert_fn_not_contains() { assert_fn_hit not_contains "$@"; }

# ---------- 3b) 反例：必须**编译失败**，且报出指定错误码 ----------
# 格式: assert_fails <example> <相对该 example 的 .rs 路径> <错误码> <说明>
# ⚠️ 所有变量引用一律用 ${var} 花括号形式：macOS 自带的是 bash 3.2，
#    它会把紧跟其后的多字节字符（如 `（`）当成变量名的一部分，
#    在 `set -u` 下直接报 unbound variable。
assert_fails() {
  local ex="$1" rel="$2" code="$3" desc="$4"
  [[ -n "$filter" && "$ex" != *"$filter"* ]] && return
  local src="examples/$ex/$rel"
  [[ -f "$src" ]] || { bad "${ex}: missing fail-case ${rel}"; return; }
  local out
  # ⚠️ 有第三方依赖的 example（见 examples/*/externs）需要 --extern / -L
  out=$(eval rustc --edition 2024 --crate-type=lib \
        "\$(bash -c 'source tools/lib.sh; rd_root >/dev/null; rd_extern_args "$ex"')" \
        "$src" 2>&1) && {
    bad "${ex}: ${desc} —— 预期编译失败，实际通过了！"; return; }
  if grep -q "$code" <<<"$out"; then ok "${ex}: ${desc}"
  else bad "${ex}: ${desc} —— 没有报出 ${code}"; fi
}

section "断言"

# ---- ch01: 借用检查 = CFG 数据流；--emit=mir 是借用检查之后的产物 ----
assert_contains ch01-borrow ".mir" '_2 = &_1'            "MIR 里借用是一个具名局部变量"
assert_contains ch01-borrow ".mir" '_3 = copy \(\(\*_2\)\.0' "借用被使用"
assert_contains ch01-borrow ".mir" '\(_1\.0: f64\) = const 3f64' "写发生在借用最后一次使用之后（NLL）"
assert_contains ch01-borrow ".mir" '_5 = &mut _1'        "共享借用死后的可变借用合法（nll_ok）"
assert_contains ch01-borrow ".mir" 'Vec::<u64>::push'    "branch_all_use：两个分支用掉借用后 push 合法"
assert_fails ch01-borrow "fail/E0502.rs" 'E0502' "反例必须报 E0502（借用冲突）"
assert_fails ch01-borrow "fail/use_after_conflict.rs" 'E0502' "反例：使用点在冲突之后也报 E0502"

# ---- ch02: 生命周期是约束，不是时间 ----
assert_contains ch02-lifetimes ".O3.ll" '^@without_lifetime = .*alias .*ptr @with_lifetime' "生命周期完全擦除：两个函数被折叠成 alias"
assert_contains ch02-lifetimes ".O3.ll" '^@longest_elided = .*alias .*ptr @longest'        "显式标注与省略版本生成同一个函数"
assert_fails ch02-lifetimes "fail/missing_lifetime.rs"  'E0106' "缺少生命周期标注报 E0106"
assert_fails ch02-lifetimes "fail/elision_ambiguous.rs" 'lifetime may not live long enough' "省略规则不够时报错"
assert_fails ch02-lifetimes "fail/outlives.rs"          'lifetime may not live long enough' "返回值生命周期超出输入范围"
assert_fails ch02-lifetimes "fail/over_annotated.rs"    'E0515' "过度标注生命周期反而编译不过"

# ---- ch03: variance —— 只有"能编译/不能编译"这一种证据 ----
# ⚠️ variance 没有可用的 dump 工具（见 evidence.md），也不产生代码。
# 所以这里断言的是"正例编译过 + 反例编译不过"，而不是任何输出内容。
assert_contains ch03-variance ".O3.s" '^_cov_ok:'            "协变正例编译通过"
assert_contains ch03-variance ".O3.s" '^_contrav_ok:'        "逆变正例编译通过"
assert_contains ch03-variance ".O3.s" '^_mut_invariant_ok:'  "&mut 不变性正例编译通过"
assert_fails ch03-variance "fail/invariance.rs"     'E0597' "不变性：'static 不能收缩"
assert_fails ch03-variance "fail/contravariance.rs" 'E0308' "逆变方向搞反"

# ---- ch04: Pin 是零成本的类型层约束 ----
assert_contains ch04-pin ".O3.ll" '^@plain = .*alias .*ptr @pinned' "Pin<&mut T> 与 &mut T 生成同一个函数（零成本）"
assert_contains ch04-pin ".mir"   '_0 = move _1'                    "移动就是一条 MIR move（自引用的根源）"
assert_contains ch04-pin ".O3.s"  '^_box_pin:'                      "Box::pin 需要真正的堆分配（稳定地址）"
assert_fails ch04-pin "fail/pin_requires_unpin.rs" 'E0277' "Pin::new 要求 T: Unpin"

# ---- ch05: 边界检查 ----
# raw_unchecked 真的没有检查（函数体只有 ldrb + ret）
assert_contains     ch05-bounds ".O3.s" '^_raw_unchecked:'  "raw_unchecked 存在"
# safe / checked_unchecked 的热路径相同：都是 cmp + b.hs + ldrb + ret
assert_contains     ch05-bounds ".O3.s" 'panic_bounds_check' "safe 走 panic_bounds_check（带索引与长度）"
assert_contains     ch05-bounds ".O3.s" 'panicking5panic'     "checked_unchecked 走 panic（带 assertion 消息）—— 失败路径不同"
# provably_in_bounds：LLVM 证明了不越界，检查被消除
assert_contains     ch05-bounds ".O3.s" 'and\s+x[0-9]+, x[0-9]+, #0x3' "provably_in_bounds 用 and #0x3 取模"

# ---- ch05: 实战（树/图容器）—— arena 索引 vs Rc<RefCell> ----
assert_contains ch05-project-tree ".O3.s" '^_arena_depth:'                     "arena 版本存在"
assert_contains ch05-project-tree ".O3.s" 'madd\s+x[0-9]+, x[0-9]+, x[0-9]+, x[0-9]+' "arena 寻址是一条乘加（索引 * 大小 + base）"
assert_contains ch05-project-tree ".O3.s" 'panic_already_mutably_borrowed'     "Rc<RefCell> 版本带运行时借用检查的 panic 路径"
assert_contains ch05-project-tree ".O3.s" '9drop_slow'                          "Rc 版本带引用计数归零的慢路径"
assert_contains ch05-project-tree ".O3.s" '__Unwind_Resume'                    "上面两条带来异常安全代码"

# ---- ch06: 关联类型 vs 泛型参数 ----
assert_contains ch06-associated-types ".O3.s" '^_conv_with_annotation:' "泛型参数版本：返回类型确定 impl，无需标注"
assert_contains ch06-associated-types ".O3.s" '^_conv_explicit:'        "泛型参数版本：也可以显式指定 Conv::<f64>"
assert_contains ch06-associated-types ".O3.s" '^_use_container:'       "关联类型版本：调用点无需类型标注"
assert_fails ch06-associated-types "fail/ambiguous.rs"       'E0283' "泛型参数导致类型推断歧义"
assert_fails ch06-associated-types "fail/conflicting_impl.rs" 'E0119' "关联类型只能实现一次"

# ---- ch07: vtable 布局 ----
# 方法槽位从偏移 24 开始，按声明顺序排列：area=#24, name=#32
assert_contains ch07-vtable ".O3.s" 'ldr[[:space:]]+x[0-9]+, \[x[0-9]+, #24\]' "dyn 调用从 vtable 偏移 24 取第一个方法指针"
assert_contains ch07-vtable ".O3.s" 'ldr[[:space:]]+x[0-9]+, \[x[0-9]+, #32\]' "第二个方法在偏移 32（槽位按声明顺序）"
assert_contains ch07-vtable ".O3.s" '^\s*br\s+x[0-9]+'                          "dyn 调用是尾跳转(间接)"
# vtable 前 24 字节 = drop_in_place / size / align，其中 size=8 align=8（Sq 是 f64）
assert_contains ch07-vtable ".O3.s" 'asciz[[:space:]]+"\\000\\000\\000\\000\\000\\000\\000\\000\\b' "vtable 前 24 字节含 size=8/align=8"
assert_contains ch07-vtable ".O3.s" '\.quad[[:space:]]+__RNvX.*Shape4name'      "vtable 里真的有 name 的函数指针"
# ★ area 槽位：实测发现 `Sq::area` 与 `call_generic` 的函数体完全相同，
#   被 LLVM 合并了 —— 于是 vtable 的 #24 槽位指向 `_call_generic`。
#   所以这里断言的是"asciz 之后紧跟一个 .quad"，而不是具体符号名。
assert_adjacent ch07-vtable ".O3.s" 'asciz' '\.quad'                            "vtable 偏移 24 处是一个函数指针（area 槽位）"
# 三种分发方式的对照
assert_contains ch07-vtable ".O3.s" '^_call_generic:'                            "泛型：单态化后被完全内联"
assert_contains ch07-vtable ".O3.s" '^_area_boxed:'                              "Box<dyn>：多一次解引用"
assert_contains ch07-vtable ".O3.ll" '^@area_dyn = .*alias .*ptr @_RNv.*8dyn_area' "area_dyn 与 dyn_area 被折叠成同一个函数"
# dyn-compatibility（旧称 object safety）
assert_fails ch07-vtable "fail/not_dyn_compatible_generic.rs" 'E0038' "泛型方法让 trait 无法建 vtable"
assert_fails ch07-vtable "fail/not_dyn_compatible_self.rs"    'E0038' "返回 Self 让 trait 无法建 vtable"

# ---- ch08: coherence、孤儿规则与 blanket impl ----
assert_contains ch08-coherence ".O3.s" '^_use_my_display:' "本地 trait + 外部类型：合法"
assert_contains ch08-coherence ".O3.s" '^_use_wrapped:'    "外部 trait + 本地类型（newtype）：合法"
assert_contains ch08-coherence ".O3.s" '^_use_blanket:'    "blanket impl：合法（trait 是本地的）"
assert_contains ch08-coherence ".O3.s" '^_use_fundamental:' "Box<Local>（fundamental）也算本地类型"
# ★ trait 方法调用在 MIR 里已经被解析成具体 impl —— 运行期没有"查表"
assert_contains ch08-coherence ".mir" '<Vec<u64> as MyDisplay>::my_fmt' "MIR 里 trait 调用已完全解析（非动态分发）"
assert_fails ch08-coherence "fail/orphan.rs"             'E0117' "孤儿规则：外部 trait + 外部类型"
assert_fails ch08-coherence "fail/conflicting_blanket.rs" 'E0119' "blanket impl 与具体 impl 冲突"
assert_fails ch08-coherence "fail/uncovered_param.rs"    'E0210' "泛型参数没被本地类型覆盖"
assert_fails ch08-coherence "fail/orphan_vec.rs"         'E0117' "Vec<Local> 不是 fundamental：仍被孤儿规则拒绝"
# ★ E0119 有两种形态：带 `for type X`（具体类型冲突）与不带（泛型层面重叠）。
#   用不带 `for type` 的措辞断言，反过来证明"这是无限集合的重叠"。
assert_fails ch08-coherence "fail/overlapping_blanket.rs" 'E0119\]: conflicting implementations of trait .P.$' "两个 blanket impl 在泛型层面重叠"

# ---- ch09: HRTB —— `for<'a>` 是 bound 上的全称量词 ----
# ★ 最核心的一条：**省略写法在 MIR 里就是 `for<'a>`**。
#   实测 `Fn(&str) -> &str` 被展开成 `for<'a> fn(&'a str) -> &'a str {id_str}`，
#   与显式写 `for<'a>` 的版本产生**完全相同的实例化**。
assert_contains ch09-hrtb ".mir" 'elided::<for<.a> fn\(&.a str\) -> &.a str'   "省略写法 Fn(&str)->&str 展开成 HRTB（不是某个自由的 'a）"
assert_contains ch09-hrtb ".mir" 'explicit::<for<.a> fn\(&.a str\) -> &.a str' "显式 for<'a> 与省略写法产生同一个实例化"
# HRTB 可以出现在 struct 字段 / dyn / where 子句里
assert_contains ch09-hrtb ".mir" 'Parser<for<.a> fn'                          "struct 字段里的省略写法同样是 HRTB"
assert_contains ch09-hrtb ".mir" 'Box<dyn for<.a> std::ops::Fn'               "dyn 上的 HRTB（一个 trait object，不是每个 'a 一个）"
# ★ 零成本：省略版与显式版被 LLVM 折叠成同一个函数
assert_contains ch09-hrtb ".O3.ll" '^@call_elided = .*alias .*ptr @call_boxed_parser' "省略版被折叠成 alias"
assert_contains ch09-hrtb ".O3.ll" '^@call_explicit = .*alias .*ptr @call_boxed_parser' "显式版被折叠成同一个 alias"
# 反例
assert_fails ch09-hrtb "fail/too_weak.rs"            'E0597' "把 'a 提到函数签名后，F 的 bound 太弱"
assert_fails ch09-hrtb "fail/no_hrtb_for_visitor.rs" 'E0597' "带生命周期参数的 trait 没有省略简写，必须写 for<'a>"
assert_fails ch09-hrtb "fail/self_elision.rs"        'lifetime may not live long enough' "HRTB 管的是 F，不是方法自己的签名（&self 抢走输出生命周期）"

# ---- ch10: GAT —— 关联类型从"一个类型"变成"一族类型" ----
# ★ 正例：`Chunks`（**自己拥有数据**）能实现 lending iterator ——
#   这正是普通关联类型做不到的（见 fail/no_gat.rs）。
assert_contains ch10-gat ".mir" '<Chunks as LendingIter>::next' "GAT 让借自 self 的迭代器可以编译"
assert_contains ch10-gat ".mir" '<Windows<.*> as LendingIter>::next' "借自外部 's 的版本同样单态化"
# GAT 的参数不限于生命周期：类型参数版本表达"一族类型"
assert_contains ch10-gat ".mir" '<Wrapper as Family>::wrap::<u64>' "GAT 用类型参数：一族类型（一个 impl 覆盖所有 T）"
# ★ 零成本：GAT 版本与手写循环生成**同样**的向量化代码
assert_contains ch10-gat ".O3.s" '^_sum_windows:'        "GAT 迭代器版本存在"
assert_contains ch10-gat ".O3.s" '^_sum_windows_manual:' "手写循环对照版本存在"
assert_contains ch10-gat ".O3.s" 'add\.2d'               "两者都被向量化（GAT 不引入额外开销）"
# 反例
assert_fails ch10-gat "fail/no_gat.rs"       'lifetime may not live long enough' "普通关联类型表达不了借自 self 的迭代器"
assert_fails ch10-gat "fail/missing_where.rs" 'missing required bound on .Item.'  "GAT 的 where Self: 'a 是强制要求"
assert_fails ch10-gat "fail/gat_not_dyn.rs"  'E0038'                              "带 GAT 的 trait 不是 dyn compatible"

# ---- ch11: 实战 —— 五个判据在同一个 API 上的互相牵制 ----
# 关联类型版本：静态分发，两个 impl 各自单态化
assert_contains ch11-project-abstract ".mir" '<Sum as Projection>::project'    "关联类型版本：Sum 单态化"
assert_contains ch11-project-abstract ".mir" '<Count as Projection>::project'  "同一个 trait 的另一个实现：Count"
# GAT 版本：一族 key 类型，一个 impl 覆盖所有 K
assert_contains ch11-project-abstract ".mir" '<KeyedSum as KeyedProjection>::project_keyed' "GAT 版本：一族 key 类型"
# ★ 生命周期参数的 GAT：签名里 GAT 参数完全消失（只剩普通引用）
assert_contains ch11-project-abstract ".mir" 'project_borrowed\(_1: &CachedSum, _2: &\[u64\]\) -> &\[u64\]' "生命周期 GAT：MIR 签名里 GAT 参数已消失"
# ★ dyn 的真实代价：6 条指令，其中 ldr #24 + br 就是全部
#   注意断言必须限定在 `sum_dyn` **函数体内** ——
#   全文件 grep `#24` 会匹配到别处的常量偏移，说明不了任何事。
assert_fn_contains ch11-project-abstract ".O3.s" '__RNvCsrtIYgyWToU_3lib7sum_dyn' 'ldr[[:space:]]+x[0-9]+, \[x[0-9]+, #24\]' "dyn 从 vtable 偏移 24 取函数指针（与第 7 章一致）"
assert_fn_contains ch11-project-abstract ".O3.s" '__RNvCsrtIYgyWToU_3lib7sum_dyn' '^\s*br\s+x[0-9]+' "dyn 是间接尾跳转"
# ★★ 去虚化：调用点类型已知时，LLVM 把 vtable 加载整个消掉
#    `use_dyn` 函数体里 br/blr 出现 0 次 —— 这就是"dyn 只在类型真的未知时才有代价"
assert_fn_not_contains ch11-project-abstract ".O3.s" 'use_dyn' 'br\s+x[0-9]+|blr' "★ 去虚化：调用点已知类型时 dyn 的间接跳转被消掉"
# 反例
assert_fails ch11-project-abstract "fail/generic_method_kills_dyn.rs" 'E0038' "给 trait 加泛型方法 → dyn 立刻失效"
assert_fails ch11-project-abstract "fail/conflicting_blanket.rs"      'E0119' "blanket impl 与具体 impl 冲突"
assert_fails ch11-project-abstract "fail/too_weak_bound.rs"           'E0597' "生命周期 bound 作用域不对（这里需要 for<'a>）"

# ---- ch17: 实战 —— 并发任务池 ----
# ★ 任务类型是 `Box<dyn FnOnce() + Send + 'static>`（MIR 里逐字可见）
assert_contains ch17-project-threadpool ".mir" 'Sender::<Box<dyn FnOnce\(\) \+ Send>>::send' "任务类型：Box<dyn FnOnce() + Send>（第 7 + 12 章）"
# ★ 装箱 + unsize：`Box<F>` → `Box<dyn FnOnce() + Send>`（第 7 章的强制转换）
assert_contains ch17-project-threadpool ".mir" 'move _4 as std::boxed::Box<dyn std::ops::FnOnce\(\) \+ std::marker::Send> \(PointerCoercion\(Unsize' "★ execute 里发生 unsize 强制转换（Box<F> → Box<dyn FnOnce>）"
assert_contains ch17-project-threadpool ".mir" 'Arc::<std::sync::Mutex<std::sync::mpsc::Receiver<Box<dyn FnOnce\(\) \+ Send>>>>::new' "★ 唯一的共享可变状态：Arc<Mutex<Receiver>>（第 15 章层次 3）"
# ★ 锁的作用域只覆盖 recv —— 任务在锁之外执行
assert_contains ch17-project-threadpool ".mir" 'Mutex::<std::sync::mpsc::Receiver<Box<dyn FnOnce\(\) \+ Send>>>::lock' "worker 循环里先 lock"
assert_contains ch17-project-threadpool ".mir" 'Receiver::<Box<dyn FnOnce\(\) \+ Send>>::recv'                            "再 recv"
assert_contains ch17-project-threadpool ".mir" 'drop\(_3\)'                                                             "★ 然后立刻 drop guard（任务在锁外执行）"
assert_contains ch17-project-threadpool ".mir" 'Box<dyn FnOnce\(\) \+ Send> as FnOnce<\(\)>>::call_once'                "call_once 在 guard drop 之后"
# ★ Drop 的顺序：先 take sender，再 join
assert_contains ch17-project-threadpool ".mir" 'Option::<std::sync::mpsc::Sender<Box<dyn FnOnce\(\) \+ Send>>>::take'   "Drop 先关 channel（take sender）"
assert_contains ch17-project-threadpool ".mir" 'JoinHandle'                                                             "Drop 再 join worker"
# 反例
assert_fails ch17-project-threadpool "fail/job_not_send.rs" 'E0277' "任务必须是 Send（Rc 捕获被拒）"

# ---- ch18: Future 是惰性的 —— async fn 展开成状态机 ----
# ★ 最核心的一条：`async fn` 调用**只构造状态机**，不执行任何代码
assert_contains ch18-future ".mir" 'fn lazy\(_1: u64\) -> \{async fn body of lazy\(\)\}' "async fn 的返回类型是"状态机"（不是 u64）"
assert_contains ch18-future ".mir" 'coroutine@examples/ch18-future/src/lib.rs'         "★ 函数体被编译成 coroutine（状态机）"
# 状态机的布局：每个 await 一个 Suspend 变体
assert_contains ch18-future ".mir" 'Unresumed\(0\)'   "状态机的初始状态 Unresumed"
assert_contains ch18-future ".mir" 'Returned \(1\)'   "状态机的完成状态 Returned"
assert_contains ch18-future ".mir" 'Suspend0 \(3\)'   "第一个 await 的挂起点 Suspend0"
assert_contains ch18-future ".mir" 'Suspend1 \(4\)'   "第二个 await 的挂起点 Suspend1"
# ★ 惰性：构造与 poll 是两件不同的事
assert_contains ch18-future ".mir" 'fn make_future.*-> \{async fn body of lazy\(\)\}' "make_future 只构造（返回状态机，不 poll）"
assert_contains ch18-future ".mir" 'block_on::<\{async fn body of lazy\(\)\}>'         "run_lazy 才 poll（经由 block_on）"
# 大小：跨 await 存活的东西之和
assert_contains ch18-future ".O3.s" '^_size_of_two_awaits:' "状态机大小可测"
assert_contains ch18-future ".O3.s" '^_size_of_no_await:'   "无 await 的 async fn 状态机更小"
# ★ E0733 的措辞直接证明"状态机大小"这个概念
assert_fails ch18-future "fail/recursive_async.rs" 'E0733' "async fn 不能递归（状态机大小会无穷大）"

# ---- ch19: Pin 与 Unpin —— 自引用结构是 Pin 存在的唯一理由 ----
# ★ 零成本：Pin<&mut T> 与 &mut T 生成同一个函数
assert_contains ch19-pin ".O3.ll" '^@plain = .*alias .*ptr @pinned' "Pin<&mut T> 与 &mut T 生成同一个函数（零成本）"
assert_contains ch19-pin ".O3.s"  '^_plain = _pinned'              "汇编里两者是同一个符号"
# ★ Unpin 是 auto trait：Pin::new 需要 T: Unpin
assert_contains ch19-pin ".O3.ll" '^@pin_a_u64 = .*alias .*ptr @pin_a_plain' "u64 与普通结构体都是 Unpin（同一个函数）"
assert_fails ch19-pin "fail/pin_requires_unpin.rs" 'PhantomPinned. cannot be unpinned' "PhantomPinned 让类型 !Unpin，Pin::new 拒绝编译"
# ★ Pin 挡住移动的两条路：拿不到 &mut，也移不走
assert_fails ch19-pin "fail/move_pinned.rs" 'E0596' "拿不到 &mut T（DerefMut 未实现）"
assert_fails ch19-pin "fail/move_pinned.rs" 'E0507' "也移不走（不能 move out of dereference）"
# ★ 自引用的运行时形态：把分配到的地址本身写进自己
assert_contains ch19-pin ".O3.s" 'stp[[:space:]]+x[0-9]+, x[0-9]+, \[x[0-9]+\]' "★ 自引用 = 一条 stp：把地址和 data 一起写进去"
assert_contains ch19-pin ".O3.s" 'bl[[:space:]]+__RNvCs[A-Za-z0-9_]*___rust_alloc' "Box::pin 真的走堆分配（自引用的地址稳定性来源）"
# 读自引用：两次解引用
assert_contains ch19-pin ".O3.s" 'ldr[[:space:]]+x8, \[x8, #8\]' "read_self_ref：从 self_ref 字段（偏移 8）取地址再读"
assert_contains ch19-pin ".mir" 'get_unchecked_mut' "拿 &mut 必须走 unsafe 的 get_unchecked_mut"
# ★ 链接第 18 章：async 状态机可能是自引用的
assert_contains ch19-pin ".mir" 'field _s1: &String'  "★ 跨 await 的借用：状态机里一个字段指向另一个字段（自引用）"
assert_contains ch19-pin ".mir" 'Suspend0 \(3\): \[_s0, _s1, _s2\]' "Suspend0 变体里同时装着被借者和借用者"
# ★ 但编译器对所有 async 产物一律保守地标 !Unpin（哪怕没有借用、没有 await）
assert_fails ch19-pin "fail/future_not_unpin.rs" 'cannot be unpinned' "★ 当前 async fn future 不自动实现 Unpin（即使没有 await）"

# ---- ch20: async 中的生命周期与 Send 传染 ----
# ★ 借用跨 await → 状态机里必须留下一个引用字段
assert_contains ch20-async-lifetimes ".mir" 'field _s0: String;'  "被借用的值成为状态机字段"
assert_contains ch20-async-lifetimes ".mir" 'field _s1: &String;' "★ 跨 await 的借用：状态机里多出一个引用字段"
assert_contains ch20-async-lifetimes ".mir" 'Suspend0 \(3\): \[_s0, _s1, _s2\]' "挂起时被借者和借用者同时活着"
# 对照：借用不跨 await → 字段退化成 usize（没有引用）
assert_contains ch20-async-lifetimes ".mir" 'field _s1: usize;' "借用不跨 await：字段退化成 usize，状态机里没有引用"
# ★ 生命周期省略规则在 async 上照旧适用（反例必须报 E0106）
assert_contains ch20-async-lifetimes ".mir" 'fn longest\(' "两个输入引用 + 输出引用：必须显式标注生命周期"
assert_fails ch20-async-lifetimes "fail/ambiguous_lifetime.rs" 'E0106' "省略规则推不出来时报 E0106"
# ★ Send：锁守卫跨 await → !Send（反例）
assert_fails ch20-async-lifetimes "fail/guard_across_await.rs" 'future cannot be sent between threads safely' "锁守卫跨 await → future 不是 Send"
assert_fails ch20-async-lifetimes "fail/guard_across_await.rs" 'MutexGuard' "错误信息指出是哪个字段"
# ★ 反直觉：drop(g) 不能替代块作用域
assert_fails ch20-async-lifetimes "fail/drop_does_not_help.rs" 'maybe used later' "★ drop(guard) 不缩短变量存活区间，仍然 !Send"
# ★ 正确写法：块作用域 → 字段里只有 u64
assert_contains ch20-async-lifetimes ".mir" 'field _s0: u64;' "块作用域版本：状态机字段里没有 MutexGuard"
# ★ Send 传染：外层 future 里装着内层 future
assert_contains ch20-async-lifetimes ".mir" 'field _s0: \{async fn body of inner_send_ok\(\)\}' "★ 外层 future 的字段是内层 future（Send 沿着 .await 传染）"
assert_fails ch20-async-lifetimes "fail/send_contagion.rs" 'not implemented for .std::sync::MutexGuard' "传染：错误指向内层的原因，报在外层的调用点"
# ★ Send 与 Sync 是两个独立性质
assert_contains ch20-async-lifetimes ".mir" 'field _s0: Cell<u64>;' "Cell 字段：future 仍然 Send，但变成 !Sync"
assert_fails ch20-async-lifetimes "fail/future_not_sync.rs" 'cannot be shared between threads safely' "★ future 并非天生 !Sync：捕获 Cell 才变 !Sync"
assert_contains ch20-async-lifetimes ".O3.s" '^_check_send_good:' "编译期断言 F: Send 通过（零运行时成本）"
# ★ `assert_send` 本身在汇编里不存在：它只做类型检查，不生成任何代码
assert_not_contains ch20-async-lifetimes ".O3.s" '^_assert_send:' "assert_send 在汇编里没有符号（纯编译期）"
# ★ 但检查完的 future 仍然要被 drop —— 断言不影响生命周期
assert_contains ch20-async-lifetimes ".O3.s" '^_check_send_good:' "断言之后 future 照常 drop"

# ---- ch12: Send/Sync 是类型层检查 ----
assert_fails ch12-send-sync "fail/not_send.rs" 'E0277' "Rc<T> 不能跨线程（!Send）"
assert_fails ch12-send-sync "fail/not_sync.rs" 'E0277' "Cell<T> 不能共享引用（!Sync）"
assert_contains ch12-send-sync ".O3.s" '^_spawn_mybox:' "unsafe impl Send 之后 MyBox 可以跨线程"
# ★ `&T: Send` 需要 `T: Sync` —— 错误信息里直接写着 "required for `&Cell<u64>` to implement `Send`"
assert_contains ch12-send-sync ".O3.s" '^_cell_is_send:' "Cell<u64> 是 Send（能移动，只是不能共享）"
# ★ PhantomData 决定 auto trait：*const T 让类型 !Send，fn() -> T 让类型 Send
assert_fails ch12-send-sync "fail/phantom_not_send.rs" 'PhantomData<\*const u64>' "PhantomData<*const T> 让类型 !Send（错误信息指向 PhantomData）"
assert_contains ch12-send-sync ".O3.s" '^_spawn_safe_handle:' "PhantomData<fn() -> T> 版本是 Send（同一个 T，只差写法）"
assert_contains ch12-send-sync ".O3.s" '^_spawn_handle_field:' "精确捕获：只捕获 id 字段就能过，即使整个类型 !Send"
# ★ Sync 和 Send 是两个独立性质：MutexGuard 是 Sync 但 !Send
assert_contains ch12-send-sync ".O3.s" '^_guard_is_sync:' "MutexGuard 是 Sync（&MutexGuard 可以跨线程）"
assert_fails ch12-send-sync "fail/guard_not_send.rs" 'MutexGuard.*cannot be sent between threads safely' "MutexGuard 不是 Send（解锁必须在加锁的线程）"

# ---- ch13: Arc 的原子操作 + Mutex 的平台差异 ----
assert_contains ch13-arc-mutex ".O3.s" 'ldadd'   "Arc::clone 使用 ldadd 原子加"
assert_contains ch13-arc-mutex ".O3.s" 'dmb'     "Arc drop 路径含内存屏障"
assert_contains ch13-arc-mutex ".O3.s" 'brk'     "引用计数超过软上限后走 brk #0x1"
assert_contains ch13-arc-mutex ".O3.s" 'pal4unix4sync5mutex' "Mutex 走 pthread 实现（macOS 上不是 futex）"
assert_contains ch13-arc-mutex ".O3.s" 'Mutex8try_lock'      "try_lock 是非阻塞路径"

# ---- ch15: 共享可变状态 —— 四种设计的代价对照 ----
# ★ 同样的需求（累加），两种工具：atomic 4 条指令 vs Mutex 11+ 条 + 两次函数调用
assert_contains ch15-shared-state ".O3.s" '^_atomic_accumulate:'  "atomic 版本存在"
assert_contains ch15-shared-state ".O3.s" '^_mutex_accumulate:'   "Mutex 版本存在"
assert_contains ch15-shared-state ".O3.s" 'GLOBAL_PANIC_COUNT'    "★ Mutex 带 poison 检查（atomic 没有）"
# 单线程的 Rc<RefCell> 便宜得多，但不能跨线程
assert_contains ch15-shared-state ".O3.s" '^_local_refcell:'      "Rc<RefCell> 在单线程里可用"
assert_fails ch15-shared-state "fail/rc_refcell_thread.rs" 'E0277' "Rc<RefCell> 不能跨线程"

# ---- ch14: 消息传递 —— 发送即 move ----
assert_contains ch14-channels ".mir" 'Sender::<String>::send\(move _9, move _10\)' "send 按值拿走所有权（MIR 里是 move）"
assert_contains ch14-channels ".mir" '_10 = move _4'                               "★ 发送前先把局部变量 move 出来"
assert_contains ch14-channels ".mir" 'Arc::<String>::new\(move _5\)'                "Arc 版本：Arc::new 之后本地仍然可用"
assert_fails ch14-channels "fail/use_after_send.rs" 'E0382' "发送之后本地再用它 → E0382（发送即 move）"

# ---- ch16: atomics —— 内存序落到具体指令（AArch64） ----
assert_contains ch16-atomics ".O3.s" '^\s*ldapr\s'   "Acquire 读生成 ldapr（不是普通 ldr）"
assert_contains ch16-atomics ".O3.s" '^\s*ldar\s'    "SeqCst 读生成 ldar"
assert_contains ch16-atomics ".O3.s" '^\s*stlr\s'    "Release 写生成 stlr"
assert_contains ch16-atomics ".O3.s" 'ldaddal'       "SeqCst fetch_add 生成 ldaddal"
assert_contains ch16-atomics ".O3.s" 'casal'         "CAS 生成 casal"
# Relaxed 读**没有**内存序指令（就是普通 ldr）
assert_contains ch16-atomics ".O3.s" '^_load_relaxed:' "Relaxed 读的函数存在"

# ---- ch16: 跨架构对照（x86_64，只生成代码，不运行） ----
if [[ -z "$filter" || "ch16-atomics" == *"$filter"* ]]; then
  if target_is_installed x86_64-apple-darwin; then
    assert_contains ch16-atomics ".x86_64.s" 'lock\s+xaddq'     "x86_64 的 SeqCst fetch_add 需要 lock 前缀"
    assert_contains ch16-atomics ".x86_64.s" 'lock\s+cmpxchgq'  "x86_64 的 CAS 需要 lock cmpxchg"
    # x86 的强内存模型：Acquire 读退化成普通 movq（没有额外指令）
    assert_not_contains ch16-atomics ".x86_64.s" 'lfence|mfence|sfence' "x86_64 上这些 Ordering 不需要 fence 指令"
  else
    skip "ch16-atomics: x86_64 对照断言（x86_64-apple-darwin 未安装）"
  fi
fi

# ---- ch21: async fn in trait 的现状 ----
# ★ 泛型调用是单态化：状态机里装着具体类型的状态机
assert_contains ch21-afit ".mir" 'field _s0: \{async fn body of use_store<Mem>\(\)\}' "★ AFIT 走单态化：泛型 future 里装着具体类型的状态机"
assert_contains ch21-afit ".mir" 'fn use_store\(_1: &S\) -> \{async fn body of use_store<S>\(\)\}' "trait 方法调用点的返回类型仍是状态机"
# ★ 零分配：use_store_mem 只写判别式（没有 Box、没有 alloc）
assert_fn_not_contains ch21-afit ".O3.s" 'use_store_mem' '___rust_alloc|_rust_alloc' "★ AFIT 零分配（对照 Box<dyn Future>）"
# ★ dyn 路线：必须 Box<dyn Future>，且要 Pin 才能 await
assert_fn_contains ch21-afit ".O3.s" 'use_store_dyn' 'stp[[:space:]]+x0, x1, \[x8\]' "dyn 路线：把 (data, vtable) 胖指针存进状态机"
assert_fails ch21-afit "fail/afit_not_dyn.rs" 'E0038' "★ async fn 的 trait 不是 dyn compatible"
assert_fails ch21-afit "fail/afit_not_send.rs" 'future cannot be sent between threads safely' "★ async fn in trait 表达不了 Send"
assert_fails ch21-afit "fail/box_dyn_future_not_awaitable.rs" 'cannot be unpinned' "★ Box<dyn Future> 不能直接 await（第 19 章）"
assert_fails ch21-afit "fail/rpitit_send_at_impl.rs" 'future cannot be sent between threads safely' "★ RPITIT 的 + Send 把检查点前移到实现处"

# ---- ch24: unsafe 的边界 —— rustc 向 LLVM 承诺了什么 ----
# (1) &mut 只读时，签名与 & 完全相同（noalias + readonly）
assert_contains ch24-noalias ".O3.ll" 'define.*@sum_mut_ro\(ptr noalias nofree noundef nonnull readonly' "&mut 只读时也是 noalias+readonly"
# (2) 真的写了，readonly 就消失
assert_not_contains ch24-noalias ".O3.ll" 'define.*@sum_mut_rw\(ptr noalias nofree noundef nonnull readonly' "&mut 写入时 readonly 消失"
# (3) ★ LLVM 把两个函数合并成一个 alias
assert_contains ch24-noalias ".O3.ll" '^@sum_shared = .*alias .*ptr @sum_mut_ro' "LLVM 认定 sum_shared 与 sum_mut_ro 等价（折叠成 alias）"
# (4) ★ 两个共享引用都可能别名，却都标 noalias（LangRef: 只约束被修改的内存）
assert_contains ch24-noalias ".O3.ll" 'define.*@two_shared\(ptr noalias nofree noundef readonly.*ptr noalias nofree noundef readonly' "两个 &u64 参数都标 noalias（且这仍然健全）"
# (5) ★ noalias 让跨参数 SIMD 成为可能
assert_contains ch24-noalias ".O3.s"  'add\.2d' "add_all 跨参数向量化（noalias 是重要依据）"
# (6) ★ 同一段逻辑：安全引用带 noalias，裸指针不带 → 少一次 load
assert_contains     ch24-noalias ".O3.ll" 'define void @safe_double_add\(ptr noalias' "★ 安全引用参数带 noalias"
assert_not_contains ch24-noalias ".O3.ll" 'define void @raw_double_add\(ptr noalias'  "★ 裸指针参数不带 noalias"
# safe 版本把两次读合并成一次（w8, w9, w8, lsl #1），raw 版本必须读两次
assert_contains     ch24-noalias ".O3.s" 'add[[:space:]]+w8, w9, w8, lsl #1' "★ safe_double_add：LLVM 敢合并两次加法（凭 noalias）"
assert_fn_contains     ch24-noalias ".O3.s" 'raw_double_add' 'ldr[[:space:]]+w9, \[x1\]' "★ raw_double_add 必须重新读 *b（没有 noalias）"

# ---- ch25: MIR 中的 no_retag ----
# ⚠️ 注意断言的确切含义：stable 的 --emit=mir **只会**打印 `no_retag`
#（Rvalue::Use(_, WithRetag::No) 才打印），它来自 EraseDerefTemps 这类
# "故意不要 retag" 的 pass，**不是** Stacked Borrows 的 retag 本体。
# `no_retag` 表示这里不做 retag，不能反向当作 retag 发生的证据。
assert_contains ch25-aliasing ".mir" 'no_retag' "MIR 中出现 no_retag（EraseDerefTemps 产生的非 retag 赋值）"
# ★ UnsafeCell：唯一合法的"通过共享引用修改"
assert_contains ch25-aliasing ".O3.s" '^_cell2_roundtrip:' "UnsafeCell 版本可编译（&self 上写）"
# ★ rustc 自带的 lint 抓"把 &T 铸成 *mut T 再写"
assert_fails ch25-aliasing "fail/write_through_shared_ref.rs" 'invalid_reference_casting' "★ 通过 &T 写 → rustc 的 invalid_reference_casting lint（deny by default）"
assert_fails ch25-aliasing "fail/write_through_shared_ref.rs" 'consider using an `UnsafeCell`' "错误信息直接给出正确做法"
# ★ UnsafeCell 的用法本身在 MIR 里就是普通操作（没有额外魔法）
assert_contains ch25-aliasing ".mir" 'UnsafeCell::<u64>::get' "UnsafeCell::get 就是普通函数调用"
# ⚠️ 顺序决定一切（先建 &T 再写 = UB）只能靠 Miri 判定，见 verify-miri.sh

# ---- ch26: 何时不该用 unsafe —— 先看安全版本编译成了什么 ----
# ★ 核心：安全版本与 unsafe 版本生成**同一个函数**
assert_contains ch26-when-not-to ".O3.s" '^_sum_unchecked = _sum_safe'       "★ 循环内索引：安全版本已消除检查，unsafe 版本被折叠成 alias"
assert_contains ch26-when-not-to ".O3.s" '^_masked_unchecked = _masked_safe' "★ 掩码索引 i & 3：同样是同一个函数"
# 掩码版本真的没有检查（只有 and + ldr + ret）
assert_fn_contains ch26-when-not-to ".O3.s" 'masked_safe' 'and[[:space:]]+x[0-9]+, x[0-9]+, #0x3' "masked_safe：检查被消除，只剩掩码"
assert_fn_not_contains ch26-when-not-to ".O3.s" 'masked_safe' 'panic_bounds_check' "masked_safe 里没有 panic 路径"
# ★ 唯一有差值的情形：索引来自外部
assert_fn_contains ch26-when-not-to ".O3.s" 'get_safe' 'panic_bounds_check' "get_safe：索引来自外部 → 检查必须保留"
assert_fn_not_contains ch26-when-not-to ".O3.s" 'get_unchecked' 'panic_bounds_check' "get_unchecked：2 条指令，没有检查"
# ★ unsafe 反而更慢（第 24 章的结论）
assert_fn_contains ch26-when-not-to ".O3.s" 'safe_double_add' 'add[[:space:]]+w8, w9, w8, lsl #1' "safe_double_add：LLVM 敢合并两次加法（凭 noalias）"
assert_fn_contains ch26-when-not-to ".O3.s" 'raw_double_add'  'ldr[[:space:]]+w9, \[x1\]'          "raw_double_add：必须重新读 *b（多一条 load）"
# ★ 安全替代：split_at_mut 不需要 unsafe
assert_contains ch26-when-not-to ".O3.s" '^_split_and_sum:' "split_at_mut 是安全且等价的替代"
assert_fn_contains ch26-when-not-to ".O3.s" 'swap_two' 'panic_bounds_check' "swap_two：保留检查（i/j 是外部输入）——安全代码该有的样子"

# ---- ch22: tokio 实战 —— Send + 'static 的两半 ----
# ★ 零依赖的 example 不受影响；ch22 需要 tokio（见 examples/ch22-tokio/externs）
assert_contains ch22-tokio ".O3.s" '^_shared_counter:' "tokio::spawn 的任务可编译（锁作用域正确）"
assert_contains ch22-tokio ".O3.s" '^_blocking_work:'  "spawn_blocking 可编译（FnOnce + Send + 'static）"
assert_contains ch22-tokio ".O3.s" '^_local_task:'     "LocalSet 可以 spawn !Send 的 future"
assert_contains ch22-tokio ".O3.s" '^_rc_within_task:' "★ 不 spawn 时 Rc 在 async 里完全可用（Send 的要求来自 spawn，不是 async）"
assert_fails ch22-tokio "fail/guard_across_await.rs" 'future cannot be sent between threads safely' "★ 锁守卫跨 await → 任务不是 Send"
assert_fails ch22-tokio "fail/rc_in_spawn.rs"        'future cannot be sent between threads safely' "★ Rc 进 spawn → !Send（第 12 章）"
assert_fails ch22-tokio "fail/borrow_local.rs"       'E0373' "★ 借用局部变量 → 'static 不满足（第 12 章同一条）"

# ---- ch23: 实战 —— mini async runtime ----
# ★ executor 的核心：一个队列 + 一个循环
assert_contains ch23-project-runtime ".O3.s" '^_run_three:' "mini executor 可编译（队列 + 循环）"
assert_contains ch23-project-runtime ".O3.s" '^_run_local:' "★ block_on 路径：!Send 的 future 也能跑（不跨线程）"
# ★ 任务真的走了堆分配（Box::pin 的状态机 + Arc<Task>）
assert_fn_contains ch23-project-runtime ".O3.s" 'run_three' '___rust_alloc' "任务被 Box::pin + Arc 分配在堆上（地址稳定，第 19 章）"
# ★ 原子操作：Arc 的引用计数（waker 的 clone/wake 会配平它）
assert_fn_contains ch23-project-runtime ".O3.s" 'run_three' 'ldadd' "Arc 的引用计数走 ldadd（第 13 章）"
# ★ VecDeque 的增长路径（队列是真的队列）
assert_contains ch23-project-runtime ".O3.s" 'VecDeque.*grow' "任务队列是 VecDeque（有真实的增长路径）"

# ---------- 3c) 脚本型证据：跨 crate 探针 ----------
# 有些证据需要**两个 crate** 才能构造（孤儿规则是其中之一）：
# 同一个 .rs 里写就是本 crate，孤儿规则直接放行，证明不了任何东西。
# 格式: assert_script <example> <脚本相对路径> <说明>（脚本退出码必须为 0）
assert_script() {
  local ex="$1" rel="$2" desc="$3"
  [[ -n "$filter" && "$ex" != *"$filter"* ]] && return
  local sh="examples/$ex/$rel"
  [[ -f "$sh" ]] || { bad "${ex}: missing script ${rel}"; return; }
  local out
  if out=$(bash "$sh" 2>&1); then ok "${ex}: ${desc}"
  else bad "${ex}: ${desc} —— 脚本失败"; printf '%s\n' "$out" | sed 's/^/      /' | tail -12; fi
}

# ---- ch08（续）：跨 crate 的孤儿规则 ----
# upstream.rs 与 downstream.rs 是两个独立 crate，
# 只有这样才能证明"下游无法 blanket impl 上游的 trait"（E0210）。
assert_script ch08-coherence "cross-crate/run.sh" "跨 crate：下游无法 blanket impl 上游 trait（E0210）"

section "结果"
printf '  PASS=%d  SKIP=%d  FAIL=%d\n' "$pass" "$skipped" "$fail"
[[ "$fail" -eq 0 ]] || exit 1
