# 第 8 章：coherence、孤儿规则与 blanket impl — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> 本机工具链：`rustc 1.98.1` / `cargo 1.98.1` / LLVM 22.1.8。

## 复现命令

```bash
tools/evidence.sh ch08-coherence     # 生成 .s / .ll / .mir / .o
scripts/verify-all.sh ch08            # 11 条断言（PASS=13，含编译 + 证据生成各 1 条）
tools/objdump.sh ch08-coherence 3     # 反汇编（带 --demangle，符号可读）
```

## 关键结论与断言（11 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | 本地 trait + 外部类型：合法 | `^_use_my_display:` |
| 2 | 外部 trait + 本地类型（newtype）：合法 | `^_use_wrapped:` |
| 3 | blanket impl（trait 是本地的）：合法 | `^_use_blanket:` |
| 4 | `Box<Local>`（fundamental）算本地类型 | `^_use_fundamental:` |
| 5 | trait 调用在 MIR 里已完全解析 | `<Vec<u64> as MyDisplay>::my_fmt` |
| 6 | 孤儿规则拒绝 `impl Display for Vec<u64>` | `fail/orphan.rs` → **E0117** |
| 7 | blanket impl 与具体 impl 冲突 | `fail/conflicting_blanket.rs` → **E0119** |
| 8 | 泛型参数没被本地类型覆盖 | `fail/uncovered_param.rs` → **E0210** |
| 9 | `Vec<Local>` 不是 fundamental，仍被拒 | `fail/orphan_vec.rs` → **E0117** |
| 10 | 两个 blanket impl 在**泛型层面**重叠 | `fail/overlapping_blanket.rs` → **E0119**（无 `for type`） |
| 11 | 跨 crate：下游无法 blanket impl 上游 trait | `cross-crate/run.sh` → **E0210**（`assert_script`） |

## 四条规则的实测边界

**规则**：`impl<P1..Pn> ForeignTrait<T1..Tn> for T0` 合法，当且仅当
「`T0..Tn` 中第一个"本地类型或类型参数"是本地类型」。
（原文：the first local type or type parameter must be the local type。）

| 写法 | 判定 | 实测 |
|---|---|---|
| `impl MyDisplay for Vec<u64>` | ✅ | trait 是本地的（本地 trait 不受限） |
| `impl Display for Wrapped` | ✅ | `T0 = Wrapped` 是本地类型 |
| `impl<T> MyDebug for T where T: MyDisplay` | ✅ | trait 是本地的 |
| `impl Display for Box<Local>` | ✅ | `Box` 是 `#[fundamental]`，`Box<Local>` **算**本地类型 |
| `impl Display for &Local` | ✅ | `&T` 也是 fundamental |
| `impl<T> From<T> for Local<T>` | ✅ | `T0 = Local<T>` 是本地类型，且 `T` 被覆盖 |
| `impl<T> From<T> for Vec<Local>` | ❌ **E0210** | 第一个类型参数是 `Vec`（外部）→ 本地类型 `Local` 出现在**内部**，不算覆盖 |
| `impl Display for Vec<u64>` | ❌ **E0117** | 全外部 |
| `impl Display for Vec<Local>` | ❌ **E0117** | `Vec` 不是 fundamental |

★ 注意第 6 行与第 7 行的对照：**`Local<T>` 行，`Vec<Local>` 不行**。
规则看的不是"本地类型有没有出现"，而是"**第一个**本地类型/类型参数是不是本地类型"。

## 原始输出

### 1) 孤儿规则（`fail/orphan.rs`）

```text
error[E0117]: only traits defined in the current crate can be implemented for types defined outside of the crate
  --> examples/ch08-coherence/fail/orphan.rs:16:1
   |
16 | impl std::fmt::Display for Vec<u64> {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^--------
   |                            |
   |                            `Vec` is not defined in the current crate
   |
   = note: impl doesn't have any local type before any uncovered type parameters
   = note: define and implement a trait or new type instead
```

### 2) 覆盖规则（`fail/uncovered_param.rs`）

```text
error[E0210]: type parameter `T` must be used as an argument to some local type (e.g., `MyStruct<T>`)
  --> examples/ch08-coherence/fail/uncovered_param.rs:17:6
   |
17 | impl<T> From<T> for Vec<Local> {
   |      ^ uncovered type parameter
   |
   = note: implementing a foreign trait is only possible if at least one of the types for which it is implemented is local
   = note: only traits defined in the current crate can be implemented for a type parameter
```

### 3) E0119 的两种形态（★ 可判别的差异）

**具体类型冲突**（`fail/conflicting_blanket.rs`）—— 带 `for type X`：

```text
error[E0119]: conflicting implementations of trait `MyTrait` for type `u64`
14 | impl<T> MyTrait for T { fn f(&self) -> u64 { 0 } }
   | --------------------- first implementation here
16 | impl MyTrait for u64 { fn f(&self) -> u64 { 1 } }
   | ^^^^^^^^^^^^^^^^^^^^ conflicting implementation for `u64`
```

**泛型层面重叠**（`fail/overlapping_blanket.rs`）—— **不带** `for type X`：

```text
error[E0119]: conflicting implementations of trait `P`
3 | impl<T: Copy> P for T { fn p(&self) -> u64 { 1 } }
  | --------------------- first implementation here
4 | impl<T: Clone> P for T { fn p(&self) -> u64 { 2 } }
  | ^^^^^^^^^^^^^^^^^^^^^^ conflicting implementation
```

★ 差别的含义：后者的冲突类型集合是**无限的**（编译器说不出"对哪个类型冲突"）。
断言因此写成 `E0119\]: conflicting implementations of trait .P.$` ——
**断言"后面没有 `for type`"**，这本身就是"泛型层面重叠"的证据。

### 4) fundamental：`Box<Local>` 可以，`Vec<Local>` 不行

```rust
pub struct Local(pub u64);

impl std::fmt::Display for Box<Local> { /* ✅ 编译通过 */ }
impl std::fmt::Display for Vec<Local> { /* ❌ E0117 */ }
```

`tools/objdump.sh ch08-coherence 3` 的反汇编里能看到前者的单态化结果
（`--demangle` 把它还原成人读的形式）：

```text
<<alloc::boxed::Box<lib::Local> as core::fmt::Display>::fmt>:
```

## ★ 单态化的两个直接证据

这一章全是编译期的事，但"编译期"不等于"看不见"。两处证据：

### (1) blanket impl 的 `Self` 在 MIR 里是**具体类型**

`.evidence/ch08-coherence-lib.mir`：

```mir
fn <impl at examples/ch08-coherence/src/lib.rs:64:1: 64:33>::my_debug(_1: &T) -> String {
    ...
}

fn use_blanket() -> String {
    bb2: {
        _0 = <Vec<u64> as MyDebug>::my_debug(move _1) -> [return: bb3, unwind: bb5];
    }
}
```

`impl<T: MyDisplay> MyDebug for T` 里的 `Self` 是 `T`，
但调用点已经被**单态化**成 `<Vec<u64> as MyDebug>`。
**blanket impl 的"所有 T"在编译期收敛成了"这里用到的那个 T"。**

### (2) blanket impl 方法在 `-O` 下**没有独立符号**

实测：

```bash
grep -c 'my_debug' .evidence/ch08-coherence-lib.O3.s     # → 0
grep -c 'my_debug' .evidence/ch08-coherence-lib.O3.ll    # → 0
```

`my_debug` 一个独立函数都没生成，被完全内联进 `use_blanket` 了。

★ 注意 `.O3.ll` 里的 `!dbg` 元数据**仍然保留**了原始符号名：

```llvm
!65 = distinct !{!65, !"_RNvXs1_CsrtIYgyWToU_3libINtNtCshxvaOLs88l5_5alloc3vec3VecyENtB5_7MyDebug8my_debugB5_"}
```

—— 但这只是调试信息（`.ll` 里需要 `-C debuginfo`），
**不是函数定义**：`grep '^define'` 里没有它。

★ **这是"coherence 是纯编译期检查"的直接证据**：
孤儿规则、覆盖规则、E0119 全部在编译期判定，
**运行期没有任何残留** —— 没有查表、没有注册表、没有初始化。

### (3) 静态分发：没有 `blr`

实测四个入口函数（`use_my_display` / `use_wrapped` / `use_blanket` /
`use_fundamental`）的 `-O` 汇编里，`blr`（间接调用）出现 **0 次**：

```bash
for f in use_my_display use_wrapped use_blanket use_fundamental; do
  awk "/^_$f:/,/cfi_endproc/" .evidence/ch08-coherence-lib.O3.s | grep -c blr
done
# 0 / 0 / 0 / 0
```

对照第 7 章的 `dyn`（那里必然有 `ldr x1, [x1, #24]` + `br x1`）——
**trait 方法调用在静态分发下没有额外代价**，因为 MIR 里就已经写明了是哪个 impl。

## 为什么规则要这样设计（原文引文，均已核对）

### 官方规则（Rust Reference, *Implementations → Orphan rules*，原文）

> Given `impl<P1..=Pn> Trait<T1..=Tn> for T0`, an impl is valid only if at least
> one of the following is true:
>
> - `Trait` is a local trait
> - All of
>   - At least one of the types `T0..=Tn` must be a **local type**.
>     Let `Ti` be the **first** such type.
>   - No **uncovered** type parameters `P1..=Pn` may appear in `T0..Ti` (excluding `Ti`)

以及紧随其后的一句（★ 就是本章 fundamental 一节的依据）：

> Note that for the purposes of coherence, **fundamental types are special**.
> The `T` in `Box<T>` is **not considered covered**, and `Box<LocalType>`
> is considered local.

### 为什么要有孤儿规则（同上，原文）

> The orphan rule states that a trait implementation is only allowed if either
> the trait or at least one of the types in the implementation is defined in
> the current crate. It prevents conflicting trait implementations across
> different crates and is key to ensuring coherence.
>
> An orphan implementation is one that implements a foreign trait for a foreign
> type. If these were freely allowed, **two crates could implement the same trait
> for the same type in incompatible ways**, creating a situation where adding or
> updating a dependency could break compilation due to conflicting implementations.
>
> The orphan rule enables library authors to add new implementations to their
> traits without fear that they'll break downstream code. Without these
> restrictions, a library couldn't add an implementation like
> `impl<T: Display> MyTrait for T` without potentially conflicting with
> downstream implementations.

实测（两 crate 探针，`upstream` 定义 `trait Format`，`downstream` 试图 blanket impl）：

```text
$ bash examples/ch08-coherence/cross-crate/run.sh
== 1) 编译 upstream（独立 crate）
   OK -> libupstream.rlib

== 2) 编译 downstream（另一个 crate，--extern 链接 upstream）
error[E0210]: type parameter `T` must be used as an argument to some local type (e.g., `MyStruct<T>`)
  --> downstream.rs:24:6
   |
24 | impl<T: std::fmt::Display> Format for T {
   |      ^ uncovered type parameter
   |
   = note: implementing a foreign trait is only possible if at least one of the types for which it is implemented is local
   = note: only traits defined in the current crate can be implemented for a type parameter
```

下游**完全无法** blanket impl 上游的 trait —— 这正是引文里
"library authors can add new implementations without fear" 的落地方式。

★ **为什么必须是脚本**：如果只用一个 `rustc` 编译 `downstream.rs`，
`upstream.rs` 就是**同一个 crate**，trait 变成"本地的"，孤儿规则直接放行。
`run.sh` 编译两次（先 rlib、再 `--extern`），才能构造出真正的跨 crate 场景。
这一条已经做成断言 `assert_script`（脚本退出码必须为 0）。

### 覆盖规则的动机（负向推理）

Rust 里**不允许负向推理**（negative reasoning）：编译器不能假设"某个 trait
将来不会被实现"，因为上游随时可能加一个 impl。

所以 `impl<T> ForeignTrait for Local<T>` 之所以被接受，
是因为**只有当前 crate 能写 `Local<...>` 的 impl**（孤儿规则保证）——
"上游未来加 impl"不可能与它冲突。**唯一的负向事实来源是本地类型。**

这解释了实测里那对关键对照：

| 写法 | 判定 | 为什么 |
|---|---|---|
| `impl<T> From<T> for Local<T>` | ✅ | `Local` 是本地类型，覆盖了 `T` |
| `impl<T> From<T> for Vec<Local>` | ❌ E0210 | `Vec` 不是 fundamental → `Vec<Local>` **不算**本地类型 |

## 交叉验证（可选）

```bash
# 人读的符号名（--demangle）
tools/objdump.sh ch08-coherence 3 | grep -E '^[0-9a-f]+ <'

# MIR 里的单态化
grep -n 'MyDebug\|my_debug' .evidence/ch08-coherence-lib.mir

# blanket impl 方法没有独立符号
grep -c my_debug .evidence/ch08-coherence-lib.O3.s
```
