# 24. `unsafe` 的边界哲学：用 `unsafe` 实现安全接口

> 一句话：`unsafe` 的义务，是维护安全抽象依赖、但编译器无法全部检查的
> **安全不变量**。LLVM IR 的元数据是其中一类可观察结果，不是完整定义。
> `&T` / `&mut T` 不只是类型，它们是契约；`unsafe` 代码必须保证契约成立。

## 先把语法认清

`unsafe { ... }` 允许执行特定不安全操作；`unsafe fn` 表示调用者必须满足
文档列出的额外前置条件；`unsafe trait`/`unsafe impl` 表示实现正确性会影响
其他安全代码。`unsafe` 不关闭借用检查，也不会自动优化代码。

`unsafe` 更像一份**人工签字的责任书**，不是“关闭安全模式”的按钮。
编译器允许你完成某个动作，不代表它替你验证了地址、生命周期和别名条件。

### 放到业务里：封装系统调用和高性能缓冲区

FFI 返回裸指针时，封装层需要验证非空、对齐、长度和所有权，再构造切片；
缓冲区实现调用 `get_unchecked` 时，要先在安全入口证明索引范围。好的设计把
unsafe 压缩在很小的边界内，对外暴露普通安全 API，并把调用前置条件和内部
不变量分别写进 `# Safety` 文档与 `SAFETY` 注释。

```rust
pub fn packet(bytes: &[u8], offset: usize, len: usize) -> Option<&[u8]> {
    let end = offset.checked_add(len)?;
    (end <= bytes.len()).then(|| &bytes[offset..end])
}
```

先看安全接口能否把条件表达清楚；只有边界检查被证明确实是瓶颈时，
才值得把内部切换为裸指针，并把同样的条件保留为可审计的不变量。

这一章是全书第一次正面谈 `unsafe`。前面几章已经把 `unsafe` 的代价
以具体形态摆在眼前了：

- 第 19 章：两段生成**同样汇编**的指针代码，可能因来源和后续访问不同，
  一段仍有效、另一段已经违反别名模型；
- 第 20 章：`unsafe impl Send for MyFuture` 是替编译器撒谎，
  代价是"锁在错误的线程上被释放"；
- 第 25 章会继续：别名规则、`UnsafeCell`、`PhantomData`。

本章要回答的是**更根本**的问题：**`unsafe` 到底在向谁承诺什么？**

## 24.0 一个会让你卡住的例子

你想写一个"用 `unsafe` 换性能"的函数：

```rust
pub fn safe_double_add(a: &mut i32, b: &i32) {
    let x = *b;
    *a += x;
    *a += *b;
}

/// # Safety
/// `a` 必须有效、正确对齐且可写，`b` 必须有效、正确对齐且可读；
/// 两者在调用期间都指向已初始化的 `i32`，并且内存不重叠。
pub unsafe fn raw_double_add(a: *mut i32, b: *const i32) {
    unsafe {
        let x = *b;
        *a += x;
        *a += *b;
    }
}
```

**逻辑一字不差。** 你的直觉是："裸指针没有借用检查、没有安全检查，
应该至少一样快，说不定更快。"

实测（`aarch64-apple-darwin` / `rustc 1.98.1` / `-O` / `#[unsafe(no_mangle)]`）：

```asm
_safe_double_add:                 ; 安全引用版本
	ldr	w8, [x1]                  ; 读 *b
	ldr	w9, [x0]                  ; 读 *a
	add	w8, w9, w8, lsl #1        ; ★ 一次加法搞定：*a + *b*2
	str	w8, [x0]
	ret                               ; 总共 5 条指令

_raw_double_add:                  ; 裸指针版本
	ldr	w8, [x1]
	ldr	w9, [x0]
	add	w8, w9, w8
	str	w8, [x0]
	ldr	w9, [x1]                  ; ★ 必须重新读 *b
	add	w8, w9, w8
	str	w8, [x0]
	ret                               ; 总共 8 条指令
```

**这个裸指针版本生成了更多指令** —— 差在它多了一次内存读取。
原因不是 `unsafe` 关键字，而是函数签名从引用改成裸指针后少了别名信息。

原因是 LLVM IR 的签名（这是本章的全部要害）：

```llvm
; 安全引用
define void @safe_double_add(ptr noalias nofree noundef align 4 captures(none) dereferenceable(4) %a,
                             ptr noalias nofree noundef readonly align 4 captures(none) dereferenceable(4) %b)

; 裸指针
define void @raw_double_add(ptr noundef captures(none) %a,
                            ptr noundef readonly captures(none) %b)
```

**安全版本的 `%a` 上有 `noalias`，裸指针版本没有。**

`noalias` 的含义是"这个指针指向的内存，不会被**别的**指针改写"。
于是安全版本里 LLVM 知道：写 `*a` 不会影响到 `*b` ——
所以第二次 `*b` 不必重读，两次加法可以合并。

裸指针版本没有这个信息，LLVM 必须保守：**假设 `a` 和 `b` 可能指向同一处**，
写 `*a` 之后 `*b` 可能已经变了，必须重新读。

> **所以 `unsafe` 本身不是性能开关。** 这个版本为改用裸指针而丢失了
> `noalias` 信息，因而错过一项优化；其他 unsafe 实现也可能通过消除
> 动态检查而变快，必须针对具体表示和调用点验证。

要理解为什么会这样，得先看清 `&T` / `&mut T` 到底给 LLVM 送去了什么。

## 24.1 先把常见说法摆上桌

通常会这样概括：

- `unsafe` 允许你做五件事：解引用裸指针、调用 `unsafe fn`、
  访问 `union` 字段、访问 `static mut`、实现 `unsafe trait`；
- `unsafe` 块**不会**关掉借用检查器，它只是"打开了一扇门"；
- 正确使用 `unsafe` 需要写 `SAFETY` 注释，说明为什么它是健全的。

第三条是关键，但"说明为什么健全"这句话太笼统。
本章先从一条**可执行**的判据切入：

> **列出这段代码依赖的安全不变量；能在 LLVM IR 中观察到的部分，
> 再用元数据和生成代码交叉验证。**

要做到这一点，先得知道那些元数据长什么样。

## 24.2 编译器眼里的样子

### 24.2.1 四种元数据

`examples/ch24-noalias/src/lib.rs` 里有一组函数，把 `&T` / `&mut T`
的契约形态钉死在可断言的位置上：

| 函数 | 签名 | 元数据 |
|---|---|---|
| `sum_shared` | `&[u64]` 只读 | `noalias` + `readonly` |
| `sum_mut_ro` | `&mut [u64]` **也只读** | `noalias` + `readonly` —— **完全相同** |
| `sum_mut_rw` | `&mut [u64]` 真的写了 | `noalias`（`readonly` 消失） |
| `two_shared` | 两个 `&u64` | **两个参数都** `noalias` |
| `add_all` | `&mut [u64]` + 两个 `&[u64]` | 三个都 `noalias` |

逐个看。

**（a）`&mut` 只读时，与 `&` 的签名逐字相同**

```llvm
define noundef i64 @sum_mut_ro(ptr noalias nofree noundef nonnull readonly align 8 captures(none) %v.0, ...)

define noundef i64 @sum_mut_rw(ptr noalias nofree noundef nonnull           align 8 captures(none) %v.0, ...)
                                                                       ↑ readonly 没了
```

★ 两个函数**只有 `readonly` 一字之差**。`readonly` 是 LLVM 看**函数体**
推出来的（"这个函数没写过它"），跟参数是 `&` 还是 `&mut` **无关**。

**（b）LLVM 干脆把它们合并了**

```llvm
@sum_shared = unnamed_addr alias i64 (ptr, i64), ptr @sum_mut_ro
```

LLVM 判定 `sum_shared` 与 `sum_mut_ro` **逐位等价**，于是前者变成了后者的 alias。

**这比"并排贴两段一样的汇编"更有说服力**：不是"看起来一样"，
是**编译器认定这是同一个函数**。

**（c）两个 `&u64` 参数都标 `noalias`**

```llvm
define noundef i64 @two_shared(ptr noalias ... readonly ... %a,
                               ptr noalias ... readonly ... %b)
```

这件事第一眼像是 bug：安全 Rust 里 `f(&x, &x)` 完全合法，
两个 `&u64` **可以**指向同一块内存。两个都标 `noalias`，不是自相矛盾吗？

**不是。** 见 24.4。

### 24.2.2 `noalias` 真正的威力在跨参数时显现

`add_all(dst: &mut [u64], a: &[u64], b: &[u64])` 的 `-O` AArch64：

```asm
	ldp	q0, q1, [x9, #-32]
	ldp	q2, q3, [x9], #64
	ldp	q4, q5, [x10, #-32]
	ldp	q6, q7, [x10], #64
	add.2d	v0, v4, v0        ; ★ 三路独立 SIMD 加法 + 4 路展开
	add.2d	v1, v5, v1
	add.2d	v2, v6, v2
	add.2d	v3, v7, v3
	stp	q0, q1, [x11, #-32]
	stp	q2, q3, [x11], #64
```

LLVM 敢**同时**读 `a`、读 `b`、写 `dst`，不做任何别名检查 ——
`noalias` 是允许这类优化的重要依据；具体是否向量化还取决于循环结构、
长度关系、目标特性和优化器启发式。

这就是 `noalias` 从"元数据"变成"实际性能"的画面。

对照 `two_shared`（两个只读参数）：

```asm
_two_shared:
	ldr	x8, [x0]
	ldr	x9, [x1]
	add	x0, x9, x8
	ret
```

**没有任何别名检查** —— 因为只读，没有内存被修改（24.4 再展开）。

### 24.2.3 代价清单：`unsafe` 拿走了什么

回到 24.0。把两个版本并排看：

```llvm
; safe：a 带 noalias
%x = load i32, ptr %b, align 4
%0 = load i32, ptr %a, align 4
%reass.add = shl i32 %x, 1          ; ★ *b * 2
%1 = add i32 %0, %reass.add
store i32 %1, ptr %a

; raw：a 没有 noalias
%x = load i32, ptr %b, align 4
%0 = load i32, ptr %a, align 4
%1 = add i32 %0, %x
store i32 %1, ptr %a
%_4 = load i32, ptr %b, align 4     ; ★ 重新读 *b（可能被上面的 store 改了）
%2 = add i32 %_4, %1
store i32 %2, ptr %a
```

**LLVM 不是"不优化裸指针"，而是"没有依据去优化"。**
`noalias` 是 Rust 送给 LLVM 的**信息**，裸指针不携带这个信息，
于是 LLVM 只能假设最坏情况。

> ★ 这条与第 1、2、19 章的结论是同一个主题的不同侧面：
> **Rust 的安全保证是编译期的，而编译期的信息会变成运行时的代码质量。**
> 类型信息不只是"检查器用来拒绝你的东西"，它是**优化器的输入**。

### 24.2.4 `captures`：比 `noalias` 更细的 LLVM 属性

细看 IR 会发现还有一类元数据：

```llvm
captures(none)          ; &mut [u64]（会写）
captures(address)       ; 某些只读场景（如 &[u64] 经 black_box 传递）
```

`captures` 描述"callee 能把指针捕获到哪一步"
（`address` / `provenance` / `read_provenance` …）。

这是 LLVM IR 的参数属性；rustc 可以根据 Rust 引用和函数体的语义生成它，
Clang 等其他前端也可以生成。它描述 callee 捕获指针的方式，
是比单纯 `noalias` 更细的一类优化信息。

## 24.3 为什么必须这样设计

### 为什么 `&mut` 不只是"可写的 `&`"

看 24.2.1 的表：`&mut T` 的契约是

1. **`noalias`**：这块内存在本次调用期间**只经由这个指针访问**；
2. **可写**（`readonly` 消失，但那是 LLVM 从函数体推的，不是类型给的）。

第 1 条才是 `&mut` 真正的分量。它说的是：

> **在这个引用的生命周期里，不存在第二个能访问这块内存的途径。**

安全 Rust 保证这条（借用检查器保证 `&mut` 独占）。
而这条保证一旦成立，LLVM 就能做大量优化：
寄存器缓存、消除重复 load、向量化、重排……

**所以"`&mut` 更慢"是错的**（24.4 展开）——
`&mut` 送去的**信息更多**，优化空间更大。

### 为什么 LLVM 元数据也是 `unsafe` 必须维护的契约

因为元数据是**承诺**，而 LLVM 的优化**建立在承诺之上**。

一旦承诺是假的：

- LLVM 不会"保守一点" —— 它已经按 `noalias` 优化过了；
- 结果是**优化后的代码与你的意图不符**，而且**不可预测**；
- 这就是 UB 的实际形态：**不是崩溃，是"编译器有权做任何事"**。

第 19 章那条 `addr_of!` 的例子就是活生生的样本：
两个写法生成逐字节相同的机器码，但其中一个指针已被后续访问作废。
**契约不在代码里，只在你脑子里。**

### 为什么这条义务不能由编译器检查

有些能查（借用检查器就查了一大类），有些**查不了**：

| 义务 | 谁能查 |
|---|---|
| `&mut` 独占 | ✅ 借用检查器（第 1 章） |
| 裸指针满足调用契约 | ❌ 通常由调用者和实现共同保证 |
| 引用指向的内存已初始化 | ❌ 需要 unsafe 代码维护有效性不变量 |
| `unsafe impl Send` 属实 | ❌ 语义层面 |
| 某些别名/provenance 违规 | ⚠️ Miri 可动态检查部分路径（第 19、25 章） |

除此之外还包括对齐、生命周期、provenance、有效值、数据竞争、析构和
库 API 自身的不变量。查不了的部分只能由 unsafe 代码作者与调用者按契约
共同保证；LLVM 属性只是这些不变量在 codegen 层的一部分投影。

## 24.4 反直觉的点

### 反直觉之一：`&mut T` 不比 `&T` 慢 —— 而且两者常被折叠成同一个函数

`sum_shared`（`&[u64]`）和 `sum_mut_ro`（`&mut [u64]`，只读）的 IR **逐字相同**，
LLVM 干脆合并了：

```llvm
@sum_shared = unnamed_addr alias i64 (ptr, i64), ptr @sum_mut_ro
```

★ **`&mut` 带来的不是"readonly 的缺失"**，而是 `noalias` + "允许写入"的许可。
`readonly` 是 LLVM 看**函数体**推出来的。

**"`&mut` 更慢所以尽量用 `&`"是错的。**
两者在只读场景下生成**同一个函数**。

### 反直觉之二：`&T` 也是 `noalias`，而这**是健全的**

这是本章最需要引原文的一条。两个 `&u64` 参数**都**标 `noalias`，
而 `&x, &x` 在安全 Rust 里合法。为什么不是 UB？

因为 LLVM LangRef 对 `noalias` 的定义是：

> "This guarantee only holds for memory locations that are **modified**,
>  by any means, during the execution of the function."

即 **`noalias` 只约束"被修改"的内存**。

两个只读指针别名同一处 —— **没有任何内存被修改** —— 不触发任何义务。

**实测验证**（`tests/sb_legal.rs`，Miri 下通过）：

```rust
let v = 21u64;
let p: *const u64 = &v;
let (a, b) = unsafe { (&*p, &*p) };      // 两个共享借用指向同一处
assert_eq!(a.wrapping_add(*b), 42);      // 合法
```

★ **书里必须引这句 LangRef 原文。** `&T: noalias` 是**正确**的，不是近似 ——
这条经常被误传成"Rust 给共享引用标 noalias 是 bug"。

### 反直觉之三：`unsafe` 本身既不保证更快，也不必然更慢

24.0 已经展开。值得再钉一遍这个因果关系：

```text
安全引用 → 类型携带 noalias → LLVM 有信息 → 敢合并
裸指针   → 不携带任何别名信息 → LLVM 保守 → 必须重读
```

**裸指针默认携带的别名信息比引用少**。如果 unsafe 实现只是把引用换成
裸指针，它可能因此更难优化；如果它在已证明的前提下消除边界检查，
也可能更快。结论必须看具体代码。

> 实践推论：把 `unsafe` 当成需要明确收益与不变量的工程选择。
> 如果只是为了让借用检查器闭嘴而把引用改成裸指针，
> 你可能同时丢失优化器可利用的信息，而且不会收到任何通知。

### 反直觉之四：`unsafe` 块**不关**借用检查

```rust
let mut v = vec![1u64, 2, 3];
let r = &v[0];
unsafe { v.push(4); }        // ← 照样 E0502
```

`unsafe` 打开的是那五件事（裸指针解引用等）的**许可**，
不是"关闭规则"。`&` / `&mut` 的规则**一直生效** ——
第 1 章的借用检查器不会因为你写了 `unsafe` 就放假。

### 反直觉之五：`unsafe fn` 的调用者与实现者各有义务

`raw_double_add` 的签名是：

```rust
pub unsafe fn raw_double_add(a: *mut i32, b: *const i32)
```

它的 `# Safety` 文档列出裸指针有效性、对齐、读写权限、初始化状态、
存活期与不重叠要求。

**注意义务的分工**：调用者必须保证 `a`、`b` 非空、正确对齐、可访问、
指向已初始化的 `i32`，在调用期间有效，并满足 `a` 可写、`b` 可读且不重叠。
函数实现者则必须只在这些前置条件覆盖的范围内解引用，并保持对外承诺。

> 所以 `unsafe fn` 的文档注释里那句 `# Safety`，
> 写的其实是**给调用者的契约**。
> `unsafe fn` 把**调用前置条件**交给调用者，但不会免除实现者正确实现
> 函数体的责任。这也是为什么 `unsafe fn` 应该尽量少、并且尽量小。

## 24.5 亲手验证

```bash
tools/evidence.sh ch24-noalias
scripts/verify-all.sh ch24      # 9 条断言

# ★ &mut 只读时与 & 的签名完全一致
grep -o 'define.*@sum_mut_ro(ptr [^,]*' .evidence/ch24-noalias-lib.O3.ll
grep -o 'define.*@sum_mut_rw(ptr [^,]*' .evidence/ch24-noalias-lib.O3.ll

# ★ LLVM 把两个函数合并了
grep 'alias' .evidence/ch24-noalias-lib.O3.ll

# ★ 跨参数向量化
awk '/^_add_all:/{on=1} on{print} on&&/cfi_endproc/{exit}' .evidence/ch24-noalias-lib.O3.s

# ★ 本章新增：安全引用 vs 裸指针
grep -o 'define void @safe_double_add(ptr [^,]*' .evidence/ch24-noalias-lib.O3.ll
grep -o 'define void @raw_double_add(ptr [^,]*'  .evidence/ch24-noalias-lib.O3.ll
awk '/^_safe_double_add:/{on=1} on{print} on&&/cfi_endproc/{exit}' .evidence/ch24-noalias-lib.O3.s
awk '/^_raw_double_add:/{on=1}  on{print} on&&/cfi_endproc/{exit}' .evidence/ch24-noalias-lib.O3.s
```

**怎么算验证成功**：

1. `sum_mut_ro` 的 IR 里有 `noalias ... readonly`，
   `sum_mut_rw` 的**没有** `readonly` —— 只差一个词；
2. IR 里有 `@sum_shared = ... alias ... ptr @sum_mut_ro` —— 合并了；
3. `add_all` 的汇编里有 `add.2d` —— 跨参数 SIMD；
4. `safe_double_add` 的 IR 里 `%a` 带 `noalias`、汇编是 **5 条指令**
   （`add w8, w9, w8, lsl #1`）；
   `raw_double_add` 的 IR 里没有 `noalias`、汇编是 **8 条指令**
   （多一次 `ldr w9, [x1]`）。

## 24.6 与 unsafe 的关系

**这一章整章就是讲这个。** 收成一条可操作的清单：

### 健全性论证的模板

写 `unsafe` 时，按这四步走：

1. **指出你依赖哪条安全不变量**
   （有效性、初始化、对齐、生命周期、别名/provenance、线程安全、析构等）。
2. **论证你的 `unsafe` 代码没有破坏它的前提。**
3. **把这句话写进 `SAFETY` 注释** —— 它写给下一个人看，不是写给编译器看。
4. **能跑 Miri 就跑**（第 19 / 25 章）。它查不了全部，但能查的那部分很值。

### 三个"看起来对"的写法，其实是 UB

| 写法 | 为什么错 | 在哪一章 |
|---|---|---|
| 两个 `&mut` 指向同一处 | 破坏 `noalias`；LLVM 已按"独占"优化过 | 本章 |
| `addr_of!` 派生自引用指针后写入 | SharedReadOnly tag 被弹掉（Stacked Borrows） | 19 |
| `unsafe impl Send for MyFuture` | 锁会在错误的线程上释放 | 20 |

★ 三者的共同点：**都不会当场崩，都不会报错，都可能"碰巧能跑"。**
这就是 UB 的形态。

### 什么时候 `unsafe` 是合理的

本章不给"永远别用 unsafe"这种结论（第 26 章专门讨论）。
但可以给一条判据：

> **`unsafe` 合理的场合，是"你比编译器知道得更多，而且能证明"** ——
> 比如你已经用一个不变量保证了独占（`Vec` 的 `set_len` 依赖"容量够"），
> 或者你正在实现一个**抽象的边界**（`Vec` / `Mutex` / `Arc` 都是这么写的）。

反过来：

> **"为了让借用检查器闭嘴"几乎从来不是理由** ——
> 因为它同时也让优化器闭嘴（24.4 反直觉之三）。

## 24.7 小结

- **`&T` / `&mut T` 是写进 LLVM IR 的契约**：
  `&mut` → `noalias`；只读时连 `readonly` 都会出现，与 `&` **逐字相同**。
- **`&mut T` 不比 `&T` 慢**：只读场景下 LLVM 把两者折叠成同一个函数
  （`@sum_shared = ... alias ... ptr @sum_mut_ro`）。
- **`&T` 也是 `noalias`，而且这是健全的**：
  LangRef 说 `noalias` "only holds for memory locations that are **modified**"。
  只读指针别名同一处，不触发任何义务。
- **`noalias` 的威力在跨参数时显现**：它是 `add_all` 能直接向量化的
  重要依据之一，但具体代码生成仍受目标与优化器影响。
- **★ `unsafe` 不是性能开关**：这个裸指针版本因为缺少引用携带的
  别名信息而错过优化；其他具体场景仍需单独测量。
  同一段逻辑，安全引用 5 条指令、裸指针 8 条指令 ——
  差的那一次 load 就是 `noalias` 的价格。**而且这份损失不报错、不警告。**
- **`unsafe` 的义务是维护完整的安全不变量**；LLVM 元数据只是其中一部分
  可观察结果。查不了的部分（别名、初始化、有效性、`Send` 属实等）只能由你保证，
  写进 `SAFETY` 注释，并尽量用 Miri 检查。
- **`unsafe fn` 的 `# Safety` 写的是给调用者的前置条件**；实现者仍负责
  在这些条件下正确实现函数体。所以 `unsafe fn` 应该尽量少、尽量小。

下一章进入 `unsafe` 里最容易出错的一块：**别名规则**。
我们会看到 `UnsafeCell` 为什么必须存在、`PhantomData` 在做什么、
以及为什么"MIR 里看不到 retag"这件事本身就是一个值得讲清楚的坑。
