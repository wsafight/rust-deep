# 9. 高阶 trait bound（HRTB）

> 一句话：`for<'a>` 不是"某个生命周期"，而是 bound 上的**全称量词**。
> 它和 `fn f<'a, ...>` 的差别不是风格，而是 `'a` 的**作用域**——
> 前者要求 `F` 同时适用于每一个 `'a`；后者让一次函数调用选定一个 `'a`，
> 函数体里的 `F` 只得到针对这个 `'a` 的保证。

## 先把语法认清

`for<'a> F: Fn(&'a str) -> &'a str` 表示同一个 `F` 必须适用于调用者
临时选择的任意生命周期。它常以省略形式 `F: Fn(&str) -> &str` 出现。
与函数自身声明 `'a` 不同，HRTB 把选择生命周期的权力交给每一次调用。

一句话抓住它：`fn f<'a>` 是**每次调用先定一个 `'a`**，`for<'a>` 则要求
**同一个 `F` 对每个 `'a` 都接得住**。位置一换，函数体能使用的保证就变了。

### 放到业务里：请求中间件与零拷贝解析器

服务器会复用同一个中间件处理许多生命周期互不相关的请求；解析器也常从
每次传入的字节缓冲区借出切片。若回调只适用于某个固定 `'a`，它就无法被
安全复用。HRTB 表达“无论这次请求活多久，我都能处理，并且返回值只借自
这次输入”。

```rust
fn run_parser<F>(f: F, packet: &[u8]) -> &[u8]
where
    F: for<'a> Fn(&'a [u8]) -> &'a [u8],
{
    f(packet)
}
```

`run_parser` 不替回调挑一个固定生命周期；每次收到新 packet，回调都得接得住。

## 9.0 一个会让你卡住的例子

你写了一个把闭包存起来的小结构：

```rust,ignore
pub struct Parser<F: Fn(&str) -> &str>(pub F);

impl<F: Fn(&str) -> &str> Parser<F> {
    pub fn parse(&self, s: &str) -> &str { (self.0)(s) }
}
```

```text
error: lifetime may not live long enough
 --> src/lib.rs:4:44
  |
4 |     pub fn parse(&self, s: &str) -> &str { (self.0)(s) }
  |                  -         -               ^^^^^^^^^^^ method was supposed to return data with
  |                  |         |                          lifetime `'2` but it is returning data
  |                  |         |                          with lifetime `'1`
  |                  |         let's call the lifetime of this reference `'1`
  |                  let's call the lifetime of this reference `'2`
help: consider introducing a named lifetime parameter
  |
4 |     pub fn parse<'a>(&self, s: &'a str) -> &'a str { (self.0)(s) }
```

编译器给的修复建议（加 `<'a>`）**确实能让它过**，但你会隐约觉得不对：
`F: Fn(&str) -> &str` 里的 `&str` 明明没写生命周期，
为什么返回值会被绑到 `&self` 上？

更让人困惑的是另一个场景。下面这段**看起来等价**的代码，一个过一个不过：

```rust,ignore
// ❌ 编译不过
pub fn weak<'a, F: Fn(&'a str) -> &'a str>(f: F, s: &'a str) -> usize {
    let local = String::from("local");
    f(&local).len() + f(s).len()
}

// ✅ 编译通过
pub fn strong<F>(f: F) -> usize
where F: for<'a> Fn(&'a str) -> &'a str
{
    let local = String::from("local");
    f(&local).len()
}
```

**函数体几乎一样，`weak` 却报 `E0597: `local` does not live long enough`。**

这一章要说清楚：`for<'a>` 到底放在哪、量的是什么，
以及为什么"省略写法"和"显式标注 `'a`"**不是同一件事**。

## 9.1 先把常见说法摆上桌

通常会这样概括：

- **HRTB**（higher-ranked trait bound）写作 `for<'a> Trait<'a>`，
  表示"对任意生命周期 `'a` 都满足"；
- 它主要用于闭包：`Fn(&str) -> &str` 是 `for<'a> Fn(&'a str) -> &'a str` 的简写；
- 常见于需要"接受任意生命周期的引用"的 API。

这些都对，但"对任意生命周期都满足"这句话太抽象——
它没说清"任意"到底是相对于**谁**的，也没解释为什么 `weak` 会失败。

本章把它换成一个可以**在 MIR 里读出来**的说法。

## 9.2 编译器眼里的样子

### 9.2.1 省略写法在 MIR 里**就是** `for<'a>`

先看两个函数，一个用省略写法，一个显式写 `for<'a>`：

```rust
pub fn elided<F: Fn(&str) -> &str>(f: F) -> usize { f("hello").len() }
pub fn explicit<F: for<'a> Fn(&'a str) -> &'a str>(f: F) -> usize { f("hello").len() }
```

调用点（`.evidence/ch09-hrtb-lib.mir`，第 98 与 110 行）：

```mir
_0 = elided::<for<'a> fn(&'a str) -> &'a str {id_str}>(id_str) -> [return: bb1, unwind continue];
_0 = explicit::<for<'a> fn(&'a str) -> &'a str {id_str}>(id_str) -> [return: bb1, unwind continue];
```

**两行的实例化参数逐字相同，都带 `for<'a>`。**

这就是本章最直接的一条证据：省略规则把 `Fn(&str) -> &str`
展开成 **`for<'a> Fn(&'a str) -> &'a str`** ——
不是 `'static`，也不是某个自由的 `'a`。

> 官方文档里"`Fn(&str) -> &str` 是简写"这句话，
> 在 MIR 里是**可以逐字读到的**。这条也做成了断言：
> `assert_contains ch09-hrtb ".mir" 'elided::<for<.a> fn\(&.a str\) -> &.a str'`。

### 9.2.2 四种位置上的 HRTB

HRTB 不只出现在函数 bound 上。实测（同一份 `.mir`）：

```mir
; 1) 函数 bound（省略写法）—— 同上
_0 = elided::<for<'a> fn(&'a str) -> &'a str {id_str}>(id_str)

; 2) struct 字段（第 211 行）
let _1: Parser<for<'a> fn(&'a str) -> &'a str {id_str}>;

; 3) dyn（第 365–374 行）
fn boxed_parser() -> Box<dyn for<'a> Fn(&'a str) -> &str> {
    _1 = Box::<for<'a> fn(&'a str) -> &'a str {id_str}>::new(id_str);
    _0 = move _1 as Box<dyn for<'a> std::ops::Fn(&'a str) -> &str>
                   (PointerCoercion(Unsize, Implicit));
}

; 4) where 子句（`T: for<'a> Visitor<'a>`）
fn total_visits(_1: &T) -> usize
```

★ 第 2 条值得留意：**struct 字段里没有"省略规则"**——
省略规则只作用于函数签名。但 `Parser<F: Fn(&str) -> &str>`
的 MIR 里**照样**是 `for<'a>`。

因为省略规则在这里的职责只是"把 `&str` 补全成 `&'a str`"，
补出来的 `'a` 没有别的地方可放，**只能变成 `for<'a>`**。
（这也是为什么 `Parser<F: Fn(&'a str) -> &'a str>` 会报
"undeclared lifetime"——`'a` 无处声明。）

★ 第 3 条：`Box<dyn for<'a> Fn(...)>` 里的 `for<'a>`
是 **trait object 类型的一部分**，意思是"这个 object 对每个 `'a` 都能用"，
**不是**"对每个 `'a` 有一个 object"。

### 9.2.3 生命周期的作用域：`for<'a>` vs `fn f<'a>`

回到 9.0 那个"一个过一个不过"的例子。把两者的语义并排写出来：

```rust,ignore
// 一次调用选定一个 'a；F 只需满足这个 'a
pub fn weak<'a, F: Fn(&'a str) -> &'a str>(f: F, s: &'a str) -> usize { ... }

// 全称量词：对**每一个** 'a 都成立
pub fn strong<F: for<'a> Fn(&'a str) -> &'a str>(f: F) -> usize { ... }
```

| | `weak<'a, ...>` | `strong<for<'a> ...>` |
|---|---|---|
| `'a` 如何生效 | 每次调用选定一个，函数体必须按该选择工作 | `F` 同时对所有 `'a` 成立 |
| `f` 能接受什么 | 只能接受活得 ≥ `'a` 的 `&str` | 任意生命周期的 `&str` |
| 函数内部造的 `local` | ❌ 活不过 `'a` | ✅ 可以 |

实测 `weak` 的报错：

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

错误信息把话说得很明白：**`argument requires that `local` is borrowed for `'a`**。
`'a` 是调用者给的，`local` 是函数内部的，前者必然比后者长。

> **一句可携带的判据**：
> 问自己"我能不能用自己临时造的字符串去调这个闭包？"
> 能 → 你需要 `for<'a>`；不能 → 普通的生命周期参数就够了。

### 9.2.4 零成本：省略版与显式版是同一个函数

`.evidence/ch09-hrtb-lib.O3.ll`：

```llvm
@call_elided   = unnamed_addr alias i64 (), ptr @call_boxed_parser
@call_explicit = unnamed_addr alias i64 (), ptr @call_boxed_parser
@use_parser    = unnamed_addr alias i64 (), ptr @call_strong
```

LLVM 判定这三个函数**逐位等价**，把前两个直接变成 alias。
**省略写法不产生任何额外代码**——它和显式写 `for<'a>` 是同一个函数。

再看单态化之后的汇编（`-O`）：

```asm
_call_strong:
	bl	__RNvCs1njKG4L9aB3_7___rustc35___rust_no_alloc_shim_is_unstable_v2
	mov	w0, #5        ; "local" 的长度，编译期已知
	ret

_call_boxed_parser:
	mov	w0, #5        ; "hello" 的长度
	ret
```

整个计算被折叠成一个常量。**HRTB 没有任何运行时成本**——
它只是一个编译期的量词。

### 9.2.5 函数指针版的代价：`ldr` + `br`

有一个地方**确实**有代价：`dyn` 或函数指针。

```asm
_use_cb:
	ldr	x3, [x0]      ; 从 struct 里取出函数指针
	mov	x0, x1        ; 参数前移（原来是 &self, &str）
	mov	x1, x2
	br	x3            ; 间接尾跳转
```

对照第 7 章 `dyn` 的 `ldr x1, [x1, #24]` + `br x1` ——
**同一类代价**：一次加载 + 一次间接跳转。
区别只在函数指针的位置（struct 的第 0 个字段 vs vtable 的第 4 个槽位）。

> 注意：这个代价来自**动态分发**，不来自 HRTB。
> `call_strong` / `call_visitor` 这些泛型版全都被完全内联了。

## 9.3 为什么必须这样设计

### 为什么不能"自动把 `'a` 变成 `for<'a>`"

编译器**做不到**这个"聪明"的推断，因为两种语义的**调用者不同**：

- `for<'a>`：`f` 必须对所有 `'a` 都工作，调用者可以随便挑；
- `'a` 在签名上：调用者挑了一个 `'a`，`f` 只需要对**这一个**工作。

后者的约束**更弱**。如果编译器自动把 `weak` 的 `'a` 提升成 `for<'a>`，
那它就**擅自加强了**调用者对 `f` 的要求——
一个只实现了 `Fn(&'short str)` 的闭包本来能用，现在就编译不过了。

**这是 Rust "不做隐式加强"原则的又一例**：
把 `'a` 放在哪里，语义就不同，编译器不会替你选。

### 为什么省略规则要展开成 `for<'a>` 而不是别的

因为**展开成 `'static` 会让绝大多数代码编译不过**：

```rust
pub fn elided<F: Fn(&'static str) -> &'static str>(f: F) -> usize { f("hello").len() }
//                       ^^^^^^^^^^^^^^ 那么 f 只能接受字面量
```

而展开成"某个自由的 `'a`"又无处声明（9.2.2 提到的那条）。
**`for<'a>` 是唯一"最不意外"的选择**：
它让 `f` 对任何生命周期的输入都能工作——这正是写 `Fn(&str) -> &str` 的人想要的。

### 为什么 `&self` 会"抢走"输出生命周期

回到 9.0 那个 `parse`。省略规则的第 3 条说：

> 如果方法有 `&self` 或 `&mut self`，那么**输出**生命周期取 `self` 的。

所以 `fn parse(&self, s: &str) -> &str` 展开成：

```rust,ignore
fn parse<'s>(&'s self, s: &str) -> &'s str
```

返回值和 `s` **没关系**。而 `(self.0)(s)` 返回的偏偏是借自 `s` 的引用。

★ **这是本章最容易混淆的一点**：

- HRTB 约束的是 **`F` 这个类型**——"对每个 `'a` 都能把 `&'a str` 变成 `&'a str`"；
- `parse` 自己的签名是**另一件事**，省略规则照常生效。

**HRTB 不会"传染"给方法的返回值。** 该标的生命周期还是得标。

### 什么时候必须显式写 `for<'a>`

省略规则**没有**为"带生命周期参数的 trait"定义简写：

```rust
pub trait Visitor<'a> {
    fn visit(&self, s: &'a str) -> usize;
}
```

于是你只有两条路（实测）：

```rust,ignore
// (a) 把 'a 提成函数参数 —— 太弱，报 E0597
pub fn total_visits<'a, T: Visitor<'a>>(t: &T) -> usize {
    let local = String::from("abc");
    t.visit(&local) + t.visit("literal")
}

// (b) 显式 HRTB —— 正确
pub fn total_visits<T: for<'a> Visitor<'a>>(t: &T) -> usize { ... }
```

顺带实测到一条：**`Visitor<'_>` 在这里直接报 E0637**
（`` `'_` is a reserved lifetime name ``）——
**匿名生命周期不能用来表达"所有生命周期"**。
写不出省略形式，就只能显式写 `for<'a>`。

## 9.4 反直觉的点

### 反直觉之一：`for<'a>` 不是"某个更长的生命周期"

最常见的误解是把 `for<'a> Fn(&'a str)` 读成
"`'a` 是一个很长的生命周期"。**完全相反**：

- `for<'a>` 的意思是"**不管** `'a` 多短，都能用"；
- 对 `F` 来说，这是更强的要求：只有适用于每个 `'a` 的实现才满足；
  对函数体来说，它却提供了更强的能力，因此可以传入临时局部引用。

`weak` 里那个写在签名上的 `'a` 才是"某个特定的、由调用者决定的生命周期"。
**看起来更宽松的 `for<'a>`，实际上约束更弱。**

### 反直觉之二：省略写法**就是** HRTB，不是"某个自由的 `'a`"

很多人以为 `Fn(&str) -> &str` 里的 `&str` 会"借用某个外部的 `'a`"。
不是。MIR 里它逐字展开成 `for<'a> fn(&'a str) -> &'a str`（9.2.1）。

**所以"省略写法"和"显式标注 `'a`"是完全不同的两件事**：

| 写法 | 等价于 |
|---|---|
| `F: Fn(&str) -> &str` | `F: for<'a> Fn(&'a str) -> &'a str` |
| `fn f<'a, F: Fn(&'a str) -> &'a str>` | 每次调用选定一个 `'a`，`F` 只需支持它 |

前者是 9.0 里 `strong` 的写法（能过），后者是 `weak` 的写法（不过）。
**"省略"和"写出来"在这里恰好是相反的语义。**

### 反直觉之三：`Parser<F: Fn(&str) -> &str>` 编译得过，`Parser<F: Fn(&'a str) -> &'a str>` 编译不过

看起来只是"写全了生命周期"，但后者会报
`` use of undeclared lifetime name `'a` `` ——
因为 `'a` 无处声明。

而省略写法**自动**补出了 `for<'a>`（9.2.2 的实测）。
**在 struct 字段这个位置上，省略比写全更正确**——
这不是风格问题，是唯一能编译的写法。

### 反直觉之四：HRTB 不传染

`Parser<F: for<'a> Fn(&'a str) -> &'a str>` 里的 `for<'a>`
**只管 `F`**。`parse` 方法自己的生命周期还是得按省略规则走，
于是被 `&self` 抢走（9.3）。

很多人以为"给 `F` 加了 HRTB，方法就自动对所有生命周期都行了"。
**不是。** 量词只作用在它出现的位置。

## 9.5 亲手验证

```bash
tools/evidence.sh ch09-hrtb
scripts/verify-all.sh ch09

# ★ 最强的一条：省略写法在 MIR 里就是 for<'a>
grep -n "for<'" .evidence/ch09-hrtb-lib.mir

# 零成本：省略版与显式版折叠成同一个函数
grep 'alias' .evidence/ch09-hrtb-lib.O3.ll

# 间接调用的样子
awk '/^_use_cb:/,/cfi_endproc/' .evidence/ch09-hrtb-lib.O3.s
```

**怎么算验证成功**：

1. MIR 里 `elided::<for<'a> fn(&'a str) -> &'a str ...>` 与
   `explicit::<for<'a> fn(&'a str) -> &'a str ...>` **逐字相同**
   —— 省略写法就是 HRTB；
2. `.O3.ll` 里 `@call_elided` 与 `@call_explicit` 都是
   `alias ... ptr @call_boxed_parser` —— 零成本；
3. `fail/too_weak.rs` 报 **E0597**，且错误信息里有
   `argument requires that `local` is borrowed for `'a``；
4. `fail/no_hrtb_for_visitor.rs` 也报 **E0597**（带生命周期参数的 trait
   没有省略简写）；
5. `fail/self_elision.rs` 报 `lifetime may not live long enough`
   —— HRTB 管不了方法自己的签名。

```bash
scripts/verify-all.sh ch09      # 9 条断言
```

## 9.6 与 unsafe 的关系

这一章和 `unsafe` 关系不大——HRTB 是**纯类型层**的量化，
不涉及任何运行时保证。

但有一条间接联系值得记住：**HRTB 是 `unsafe` 代码里"生命周期契约"的表述工具**。

比如你写一个 `unsafe` 的裸指针封装，想表达
"这个回调对**任何**生命周期的引用都安全"，就得用 `for<'a>`：

```rust
pub unsafe fn with_ref<T, F>(ptr: *const T, f: F) -> usize
where
    F: for<'a> FnOnce(&'a T) -> usize,   // ← 契约：对任意 'a 都成立
{
    // SAFETY: 调用者保证 ptr 在 f 执行期间有效
    f(unsafe { &*ptr })
}
```

**用 `for<'a>` 而不是某个具体的 `'a`，就是在说"这个保证不依赖引用的存活时长"。**
把 bound 的作用域写错（把 `'a` 提到函数签名），给 `F` 的契约就变弱了——
而契约变弱，`unsafe` 的论证就不成立。

## 9.7 小结

- **`for<'a>` 是 bound 上的全称量词**：同一个 `F` 对每一个 `'a` 都成立。
  写在函数签名上的 `'a` 则在一次调用中被选定，函数体只能假设 `F`
  对这一个 `'a` 成立。
- **省略写法就是 HRTB**：MIR 里 `Fn(&str) -> &str` 逐字展开成
  `for<'a> fn(&'a str) -> &'a str`。**不是** `'static`，也不是自由的 `'a`。
- **可携带的判据**："我能不能用自己临时造的字符串去调这个闭包？"
  能 → 需要 `for<'a>`。
- **省略 vs 显式在这里恰好相反**：
  `F: Fn(&str) -> &str` 等价于 `for<'a>`（对 `F` 要求更强，对函数体更通用）；
  而 `fn f<'a, F: Fn(&'a str) -> &'a str>` 把 `'a` 交给调用者，
  **对 `f` 的要求更弱**，但**对函数体更苛刻**——它只能用活得 ≥ `'a` 的数据。
  所以"写全生命周期"不等于"更强的保证"。
- **HRTB 不传染**：它只管 `F`，方法自己的返回值生命周期该标还得标
  （`&self` 会抢走输出生命周期）。
- **带生命周期参数的 trait 没有省略简写**，必须显式 `for<'a>`；
  `Trait<'_>` 会直接报 E0637。
- **零成本**：省略版与显式版被 LLVM 折叠成同一个函数。
  有代价的是 `dyn` / 函数指针（`ldr` + `br`），那是动态分发的代价，不是 HRTB 的。

下一章我们给关联类型**加上参数**：**GAT**——
它解决的是 `Iterator` 和 `LendingIterator` 之间的那道鸿沟。
