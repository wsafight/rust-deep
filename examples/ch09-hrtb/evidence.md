# 第 9 章：HRTB —— `for<'a>` 到底在量化什么 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> 本机工具链：`rustc 1.98.1` / `cargo 1.98.1` / LLVM 22.1.8。

## 复现命令

```bash
tools/evidence.sh ch09-hrtb      # 生成 .s / .ll / .mir / .o
scripts/verify-all.sh ch09       # 9 条断言（PASS=11）
```

## 关键结论与断言（9 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | 省略写法 `Fn(&str)->&str` **就是** HRTB | MIR 里 `elided::<for<'a> fn(&'a str) -> &'a str` |
| 2 | 显式 `for<'a>` 产生**同一个**实例化 | MIR 里 `explicit::<for<'a> fn(&'a str) -> &'a str` |
| 3 | struct 字段里的省略写法同样是 HRTB | MIR 里 `Parser<for<'a> fn` |
| 4 | `dyn` 上的 HRTB 是**一个** trait object | MIR 里 `Box<dyn for<'a> std::ops::Fn` |
| 5 | 省略版被 LLVM 折叠成 alias | `.O3.ll` 的 `@call_elided = ... alias ... @call_boxed_parser` |
| 6 | 显式版被折叠成**同一个** alias | `.O3.ll` 的 `@call_explicit = ... alias ... @call_boxed_parser` |
| 7 | 把 `'a` 提到函数签名后，`F` 的 bound 太弱 | `fail/too_weak.rs` → **E0597** |
| 8 | 带生命周期参数的 trait 没有省略简写 | `fail/no_hrtb_for_visitor.rs` → **E0597** |
| 9 | HRTB 管的是 `F`，不是方法自己的签名 | `fail/self_elision.rs` → `lifetime may not live long enough` |

## ★ 最强的一条证据：省略写法在 MIR 里就是 `for<'a>`

`.evidence/ch09-hrtb-lib.mir`（第 98、110、183 行）：

```mir
_0 = elided::<for<'a> fn(&'a str) -> &'a str {id_str}>(id_str) -> [return: bb1, unwind continue];
_0 = explicit::<for<'a> fn(&'a str) -> &'a str {id_str}>(id_str) -> [return: bb1, unwind continue];
_0 = strong::<for<'a> fn(&'a str) -> &'a str {id_str}>(id_str) -> [return: bb1, unwind continue];
```

三个函数里，两个写的是省略形式（`Fn(&str) -> &str`），
但 MIR 里的实例化参数**逐字相同**，都带 `for<'a>`。

> **这直接回答了"省略规则把 `&str` 展开成什么"这个问题：**
> 不是 `'static`，不是某个自由的 `'a`，而是 **`for<'a>`**。
> 官方文档里"`Fn(&str) -> &str` 是 `for<'a> Fn(&'a str) -> &'a str` 的简写"
> 这句话，在 MIR 里是**可以逐字读到的**。

## HRTB 出现的四种位置（全部实测）

```mir
; 1) 函数 bound（省略写法）—— 同上
_0 = elided::<for<'a> fn(&'a str) -> &'a str {id_str}>(id_str)

; 2) struct 字段（第 211 行）
let _1: Parser<for<'a> fn(&'a str) -> &'a str {id_str}>;

; 3) dyn（第 365–374 行）
fn boxed_parser() -> Box<dyn for<'a> Fn(&'a str) -> &str> {
    _1 = Box::<for<'a> fn(&'a str) -> &'a str {id_str}>::new(id_str);
    _0 = move _1 as Box<dyn for<'a> std::ops::Fn(&'a str) -> &str> (PointerCoercion(Unsize, Implicit));
}

; 4) where 子句（第 263 行，`T: for<'a> Visitor<'a>`）
fn total_visits(_1: &T) -> usize
```

★ 第 3 条的措辞值得注意：`Box<dyn for<'a> Fn(...)>` 里的 `for<'a>`
是 **trait object 类型的一部分**，表示"这个 object 对每个 `'a` 都能用"，
**不是**"对每个 `'a` 有一个 object"。

## 零成本：省略版与显式版是同一个函数

`.evidence/ch09-hrtb-lib.O3.ll`（第 8–10 行）：

```llvm
@call_elided   = unnamed_addr alias i64 (), ptr @call_boxed_parser
@call_explicit = unnamed_addr alias i64 (), ptr @call_boxed_parser
@use_parser    = unnamed_addr alias i64 (), ptr @call_strong
```

LLVM 判定 `call_elided` / `call_explicit` / `call_boxed_parser`
**逐位等价**，把前两个直接变成 alias。
**省略写法不产生任何额外代码** —— 它就是同一个函数。

## 间接调用：`fn` 指针版本的代价

`use_cb`（`Cb<'a, fn(&'a str) -> usize>`）的 `-O` 汇编：

```asm
_use_cb:
	ldr	x3, [x0]      ; 从 struct 里取出函数指针
	mov	x0, x1        ; 参数前移（原来是 &self, &str）
	mov	x1, x2
	br	x3            ; 间接尾跳转
```

对照第 7 章 `dyn` 的 `ldr x1, [x1, #24]` + `br x1` ——
**同一类代价**（一次加载 + 一次间接跳转），
只不过这里的函数指针在 struct 的第 0 个字段，而不是 vtable 的第 4 个槽位。

★ 而 `call_strong` / `call_visitor` 这些**泛型版**（单态化后）反而更简单：

```asm
_call_strong:
	bl	__RNvCs1njKG4L9aB3_7___rustc35___rust_no_alloc_shim_is_unstable_v2
	mov	w0, #5        ; "local" 的长度，编译期已知
	ret

_call_boxed_parser:
	mov	w0, #5        ; "hello" 的长度
	ret
```

**整个计算被折叠成一个常量** —— 这就是 HRTB 不引入任何运行时成本的样子。

## 反例一：`fail/too_weak.rs`（生命周期 bound 的作用域不对）

```rust
pub fn weak<'a, F: Fn(&'a str) -> &'a str>(f: F, s: &'a str) -> usize {
    let local = String::from("local");
    f(&local).len() + f(s).len()
}
```

```text
error[E0597]: `local` does not live long enough
  --> examples/ch09-hrtb/fail/too_weak.rs:23:7
   |
20 | pub fn weak<'a, F: Fn(&'a str) -> &'a str>(f: F, s: &'a str) -> usize {
   |             -- lifetime `'a` defined here
21 |     let local = String::from("local");
   |         ----- binding `local` declared here
23 |     f(&local).len() + f(s).len()
   |     --^^^^^^-
   |     | |
   |     | borrowed value does not live long enough
   |     | argument requires that `local` is borrowed for `'a`
24 | }
   | - `local` dropped here while still borrowed
```

**`'a` 写在函数签名上 = 由调用者选。** 于是 `f` 只能用
"活得 ≥ `'a`"的字符串去调，而函数内部的 `local` 显然活不了那么久。

对照 `src/lib.rs` 的 `strong`（`F: for<'a> Fn(&'a str) -> &'a str`）——
**完全一样的函数体，编译通过**。差别只在量词：

| | 量词 | 含义 | `local` 能用吗 |
|---|---|---|---|
| `fn weak<'a, F: Fn(&'a str)->&'a str>` | `'a` 作用于整次调用 | `F` 只保证支持这一个 `'a` | ❌ |
| `fn strong<F: for<'a> Fn(&'a str)->&'a str>` | 全称（`for<'a>`） | 对**每个** `'a` 都成立 | ✅ |

## 反例二：`fail/no_hrtb_for_visitor.rs`（省略规则表达不了）

```rust
pub trait Visitor<'a> {
    fn visit(&self, s: &'a str) -> usize;
}

pub fn total_visits<'a, T: Visitor<'a>>(t: &T) -> usize {   // ← 太弱
    let local = String::from("abc");
    t.visit(&local) + t.visit("literal")
}
```

同样报 **E0597**。正确写法（`src/lib.rs` 的 `total_visits`）是：

```rust
pub fn total_visits<T>(t: &T) -> usize where T: for<'a> Visitor<'a>
```

★ 顺带实测到的一条：**`Visitor<'_>` 在这里直接报 E0637**
（`` `'_` is a reserved lifetime name ``）——
**匿名生命周期不能用来表达"所有生命周期"**。
写不出省略形式，就只能显式写 `for<'a>`。

## 反例三：`fail/self_elision.rs`（HRTB 管的是 `F`，不是方法签名）

```rust
pub struct Parser<F: for<'a> Fn(&'a str) -> &'a str>(pub F);

impl<F: for<'a> Fn(&'a str) -> &'a str> Parser<F> {
    pub fn parse(&self, s: &str) -> &str { (self.0)(s) }
}
```

```text
error: lifetime may not live long enough
   |
 4 |     pub fn parse(&self, s: &str) -> &str { (self.0)(s) }
   |                  -         -               ^^^^^^^^^^^ method was supposed to return data with lifetime `'2` but it is returning data with lifetime `'1`
   |                  |         |
   |                  |         let's call the lifetime of this reference `'1`
   |                  let's call the lifetime of this reference `'2`
```

`F` 明明已经是 `for<'a>`，为什么还不过？

因为**省略规则**遇到 `&self` 时，输出生命周期**一律取 `&self` 的那个**：

```rust
fn parse(&self, s: &str) -> &str
//        ^^^^^  '1        ^^^^ '1（被 &self 抢走了，不是 s 的）
```

而 `(self.0)(s)` 返回的是**借自 `s`** 的引用。

★ **关键区分**：
- HRTB 约束的是 **`F` 这个类型**（"对每个 `'a` 都能把 `&'a str` 变成 `&'a str`"）；
- `parse` 自己的签名是**另一件事**，省略规则照常生效。

正确写法：`pub fn parse<'a>(&self, s: &'a str) -> &'a str`。

## 交叉验证（可选）

```bash
# 省略写法 = HRTB（最强的一条）
grep -n "for<'" .evidence/ch09-hrtb-lib.mir

# 零成本
grep 'alias' .evidence/ch09-hrtb-lib.O3.ll

# 间接调用的样子
awk '/^_use_cb:/,/cfi_endproc/' .evidence/ch09-hrtb-lib.O3.s
```
