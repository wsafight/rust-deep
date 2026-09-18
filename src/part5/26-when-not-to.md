# 26. 何时**不该**用 `unsafe`

> 一句话：**先看安全版本编译成了什么。**
> 在你花力气论证健全性之前，先确认 `unsafe` 真的换来了东西 ——
> 因为很多时候，编译器**已经给了你**。

前两章讲了 `unsafe` 的义务（第 24 章）和它最难维持的那一条（第 25 章）。
这一章收尾，给一条**可以照着走**的判据 ——
不是"尽量别用"，而是一个三步流程。

## 26.0 一个会让你卡住的例子

你想优化一个求和循环：

```rust
pub fn sum_safe(v: &[u64]) -> u64 {
    let mut s = 0u64;
    for i in 0..v.len() { s = s.wrapping_add(v[i]); }
    s
}
```

`v[i]` 每次都带边界检查。你想起"边界检查有开销"，于是写：

```rust
pub fn sum_unchecked(v: &[u64]) -> u64 {
    let mut s = 0u64;
    for i in 0..v.len() {
        // SAFETY: i < v.len() 由循环条件保证
        s = s.wrapping_add(unsafe { *v.get_unchecked(i) });
    }
    s
}
```

**你付出的代价**：一段 `unsafe`、一条 `SAFETY` 论证、
以及"以后有人改这个循环时可能破坏不变量"的风险。

**你换来的东西**，实测（`-O` / `aarch64-apple-darwin` / rustc 1.98.1）：

```asm
_sum_unchecked = _sum_safe
```

**零。** LLVM 判定两者**逐位等价**，把 `sum_unchecked`
折叠成了 `sum_safe` 的 alias —— 它们**是同一个函数**。

原因很简单：循环上界就是 `v.len()`，编译器**证得出** `i < v.len()`，
于是安全版本里的边界检查**早就被消除了**。

> **你写 `unsafe` 想换掉的那个检查，在安全版本里根本不存在。**

这不是特例。本章有三组对照，其中**两组**的 `unsafe` 版本
与安全版本是**同一个函数**。

## 26.1 表层解释（官方书会怎么讲）

官方书会说：

- `unsafe` 应该尽量少用，因为它把保证的责任转移给了你；
- 性能不是使用 `unsafe` 的充分理由，应该先测量；
- 常用的 `unsafe` 场景：FFI、底层数据结构、`Send`/`Sync` 的手动实现。

这些都对，但"应该先测量"这句**太软**了 ——
它没告诉你在**测量之前**能先做什么判断。

本章给的是这个：**在测量之前，先读汇编。**
它比 benchmark 更快、更确定、而且不需要构造工作负载。

## 26.2 编译器眼里的样子

### 26.2.1 三组对照（全部实测）

`examples/ch26-when-not-to/src/lib.rs`：

| 情形 | 安全版本 | `unsafe` 版本 | 差值 |
|---|---|---|---|
| 循环内索引 `0..v.len()` | 39 条指令（含向量化） | **同一个函数** | **0** |
| 掩码索引 `i & 3` | **3 条**指令 | **同一个函数** | **0** |
| 外部传入的索引 | 11 条 + panic 路径 | **2 条**指令 | 9 条 |

前两组是"检查已被消除"，第三组是"检查必须保留"。

### 26.2.2 第一组：循环内索引

```asm
_sum_unchecked = _sum_safe
```

**alias** —— LLVM 认定两者逐位等价。
（第 24 章也见过同样的手法：`@sum_shared = ... alias ... ptr @sum_mut_ro`。）

顺便：迭代器版本 `sum_iter` 也是 39 条指令，与 `sum_safe` **逐字节相同**。
**三种写法，一个函数。**

### 26.2.3 第二组：掩码索引 —— 最干净的一个例子

```rust
pub fn masked_safe(v: &[u64; 4], i: usize) -> u64 { v[i & 3] }
```

```asm
_masked_safe:
	and	x8, x1, #0x3
	ldr	x0, [x0, x8, lsl #3]
	ret
```

**3 条指令，没有任何检查。** LLVM 从 `i & 3` 直接推出了 `i & 3 < 4`。

而 `masked_unchecked`（手写 `get_unchecked`）：

```asm
_masked_unchecked = _masked_safe
```

**同一个函数。** 这个例子最适合用来建立直觉：
**"安全代码慢"这个印象，很多时候来自没看汇编。**

（第 5 章实测过同一件事：`provably_in_bounds` 的汇编里
只剩一条 `and x?, x?, #0x3`。）

### 26.2.4 第三组：外部传入的索引 —— 唯一有差值的情形

```rust
pub fn get_safe(v: &[u64], i: usize) -> u64 { v[i] }
```

```asm
_get_safe:
	cmp	x2, x1                 ; ← 边界检查
	b.hs	LBB0_2                 ; ← 越界则跳走
	ldr	x0, [x0, x2, lsl #3]
	ret
LBB0_2:                          ; cold 路径
	...
	bl	panic_bounds_check
```

**11 条指令**（热路径 4 条 + cold 路径 7 条）。

```rust
pub unsafe fn get_unchecked(v: &[u64], i: usize) -> u64 { unsafe { *v.get_unchecked(i) } }
```

```asm
_get_unchecked:
	ldr	x0, [x0, x2, lsl #3]
	ret
```

**2 条指令。** 省下了 `cmp` + `b.hs`。

★ **这是本章唯一一个 `unsafe` 真的省下东西的情形** ——
而省下的是**两条指令**，加上一段本来也不会执行的 cold 代码。

### 26.2.5 反例：`unsafe` 反而**更慢**

第 24 章的结论，这里作为"判据"的一部分再钉一次：

```rust
pub fn safe_double_add(a: &mut i32, b: &i32) { let x = *b; *a += x; *a += *b; }   // 5 条指令
pub unsafe fn raw_double_add(a: *mut i32, b: *const i32) { /* 同一段逻辑 */ }      // 8 条指令
```

安全引用的参数带 `noalias`，LLVM 敢把两次加法合并成 `*a + *b*2`；
裸指针**没有 `noalias`**，必须重新读 `*b`。

★ **`unsafe` 不是"解锁更多优化"，而是"放弃优化"** ——
而且这份损失**不报错、不警告、不出现在 profiler 里**。

### 26.2.6 安全版本的"正确替代"

手写 `unsafe` 最常见的两个动机，标准库都已经解决了：

| 你想做的事 | 手写需要 `unsafe` 因为… | 安全替代 |
|---|---|---|
| 把一个切片劈成两半 | 裸指针 + 偏移 | **`split_at_mut`** |
| 按索引取两个不同的元素 | 两个 `&mut` 指向同一数组 | **`split_at_mut`** |

```rust
pub fn swap_two(v: &mut [u64], i: usize, j: usize) -> u64 {
    if i != j && i < v.len() && j < v.len() {
        let (lo, hi) = if i < j { (i, j) } else { (j, i) };
        let (left, rest) = v.split_at_mut(hi);   // ← 安全地拿到两个不重叠的 &mut
        std::mem::swap(&mut left[lo], &mut rest[0]);
    }
    v.first().copied().unwrap_or(0)
}
```

★ 注意它的汇编里**有 `panic_bounds_check`** —— 因为 `i`、`j` 是外部输入，
检查必须保留。**这正是安全代码该有的样子**：
在有风险的地方检查，在没风险的地方（26.2.3）不检查。

## 26.3 为什么必须这样设计

### 为什么"先读汇编"比"先 benchmark"更好

因为**它回答的是一个更早的问题**。

benchmark 回答的是"哪个快"。但在 `unsafe` 这件事上，
你要先回答的是"**有没有区别**"。如果两个版本生成同一个函数，
那 benchmark 的结果必然是"没有区别"（在噪声范围内）——
**而你已经为此付出了一段 `unsafe`。**

顺序应该是：

```text
先读汇编（快、确定、不需要构造负载）
   │
   ├── 同一个函数？ → 停。没有 unsafe 的必要。
   │
   └── 真的不同？ → 再问：这个差值重要吗？→ 再 benchmark
```

### 为什么编译器经常能消除边界检查

因为边界检查的**消除条件**通常就写在你的循环里：

| 写法 | 编译器知道什么 | 检查 |
|---|---|---|
| `for i in 0..v.len() { v[i] }` | `i < v.len()` 是循环条件 | 消除 |
| `v[i & 3]`（长度 4） | `i & 3 < 4` 是位运算的**恒等式** | 消除 |
| `v[i]`（`i` 是参数） | 什么都不知道 | **保留** |

LLVM 的 `-O` 里有专门的 pass 做这件事（`LoopVectorize` 依赖它、
`InstCombine` 也推这类恒等式）。**这是 Rust 性能故事的很大一部分** ——
"安全抽象零成本"不是口号，是这些 pass 的实际工作。

> ⚠️ 但这**不是承诺**。编译器可能因为版本变化、内联决策、
> 或者周围代码的微小改动而**不再消除**某个检查。
> 所以：**不要依赖"检查会被消除"来写安全代码**，
> 更**不要**因为"我测过它被消除了"就去写 `unsafe` ——
> 后者是在用编译器的实现细节当自己的安全前提。

### 为什么标准库里的 `unsafe` 是可以接受的

因为**义务被收进了一个边界内，并且有测试和 Miri 守着**。

`split_at_mut` 内部有 `unsafe`（构造两个不重叠的 `&mut`），
但它的**契约是明确的**（"`mid <= len`"），
而且它用 `assert` 把调用者的错误变成了 panic ——
**不是 UB**。

这是"`unsafe` 的合理形态"的样板：

```text
安全接口（无前提，或者用 assert 检查前提）
    │
    └── unsafe 实现（义务在这里，且有测试/Miri 覆盖）
```

**你的代码应该消费这种边界，而不是自己造一个。**

## 26.4 反直觉的点

### 反直觉之一：`unsafe` 版本经常和安全版本是**同一个函数**

本章三组对照里，**两组**的 `unsafe` 版本被 LLVM 折叠成了安全版本的 alias。
这不是巧合，是常态：**检查消除是 LLVM 的日常工作。**

★ 推论：**在没看汇编之前，你不知道 `unsafe` 换来了什么。**
而"不知道换来了什么"就意味着"不知道值不值"。

### 反直觉之二：`unsafe` 可能**更慢**

26.2.5 已经展开。这里补一句因果关系：

```text
安全引用 → 类型携带 noalias → LLVM 有信息 → 敢优化
裸指针   → 不携带任何别名信息 → LLVM 保守 → 必须重读
```

**裸指针的语义是"我不知道"，不是"我知道得更多"。**

★ 这条推翻了一个很常见的直觉链：
"`unsafe` = 更底层 = 更快"。**错。**
`unsafe` = **编译器对你的了解更少**。

### 反直觉之三：`&mut` 不比 `&` 慢

第 24 章实测：`sum_shared`（`&[u64]`）与 `sum_mut_ro`（`&mut [u64]`，只读）
**逐字相同的 IR**，LLVM 合并了：

```llvm
@sum_shared = unnamed_addr alias i64 (ptr, i64), ptr @sum_mut_ro
```

所以"为了性能尽量用 `&`"也是错的。**两者在只读场景下是同一个函数。**

### 反直觉之四：把 `unsafe` 写小，比写对更容易

一个反直觉的实践建议：**`unsafe` 块应该尽可能小**，
而且应该被**安全的前置检查**包住。

```rust
// ❌ 大 unsafe 块：义务范围模糊
pub unsafe fn f(v: &[u64], i: usize) -> u64 {
    unsafe {
        let x = *v.get_unchecked(i);
        let y = *v.get_unchecked(i + 1);   // 这个的边界呢？
        x + y
    }
}

// ✅ 小 unsafe 块 + 明确前提
pub fn f(v: &[u64], i: usize) -> u64 {
    assert!(i + 1 < v.len(), "需要至少两个元素");   // ← 前提检查
    // SAFETY: 上面的 assert 保证 i 和 i + 1 都在界内
    unsafe { *v.get_unchecked(i) + *v.get_unchecked(i + 1) }
}
```

★ 第二个版本**接口是安全的**（不需要调用者承诺任何东西），
**义务被收进了一个 assert 里**，而且 assert 失败是 **panic 不是 UB**。

这和第 25 章 `split_at_mut` 的做法是同一个模式：
**把 `unsafe` 的前提变成一次运行时检查。**

代价是一次 `assert`（在 26.2.4 那个例子里就是 `cmp` + `b.hs`）——
**而这恰好就是你想省掉的那两条指令。** 值得想一想：
你省掉的两条指令，值不值得换一次 UB 的可能性。

### 反直觉之五：Miri 通过**不等于** sound

第 25 章讲过，这里作为收尾再强调一次：

> Miri 的规则（Stacked Borrows）**至今仍是实验性的**。
> 它报 UB 要认真查；它**不报**不能反推 sound。

所以完整的判断链是：

```text
汇编：有没有区别？
   ↓ 有
Miri：我的 unsafe 违反别名规则吗？
   ↓ 没有
论证：我的 SAFETY 注释能说服别人吗？
   ↓ 能
benchmark：这个差值对整体有影响吗？
```

**四步都过了，`unsafe` 才是值得的。**

## 26.5 亲手验证

```bash
tools/evidence.sh ch26-when-not-to
scripts/verify-all.sh ch26      # 10 条断言

# ★ 安全版本与 unsafe 版本是同一个函数
grep '^_sum_unchecked = \|^_masked_unchecked = ' .evidence/ch26-when-not-to-lib.O3.s

# ★ 掩码版本真的没有检查
awk '/^_masked_safe:/{on=1} on{print} on&&/cfi_endproc/{exit}' .evidence/ch26-when-not-to-lib.O3.s

# ★ 唯一有差值的情形
awk '/^_get_safe:/{on=1}       on{print} on&&/cfi_endproc/{exit}' .evidence/ch26-when-not-to-lib.O3.s
awk '/^_get_unchecked:/{on=1}  on{print} on&&/cfi_endproc/{exit}' .evidence/ch26-when-not-to-lib.O3.s

# ★ unsafe 反而更慢
awk '/^_safe_double_add:/{on=1} on{print} on&&/cfi_endproc/{exit}' .evidence/ch26-when-not-to-lib.O3.s
awk '/^_raw_double_add:/{on=1}  on{print} on&&/cfi_endproc/{exit}' .evidence/ch26-when-not-to-lib.O3.s
```

**怎么算验证成功**：

1. `grep` 到 **两行 alias**：
   `_sum_unchecked = _sum_safe`、`_masked_unchecked = _masked_safe`
   —— **`unsafe` 换来的东西是零**；
2. `masked_safe` 的汇编是 `and` + `ldr` + `ret`（3 条），**没有** `panic_bounds_check`；
3. `get_safe` 有 `cmp` + `b.hs` + `panic_bounds_check`（11 条），
   `get_unchecked` 只有 `ldr` + `ret`（2 条）—— **这是唯一有差值的情形**；
4. `safe_double_add` 是 5 条指令（含 `add w8, w9, w8, lsl #1`），
   `raw_double_add` 是 8 条（多一次 `ldr w9, [x1]`）—— **`unsafe` 更慢**。

## 26.6 与 unsafe 的关系

**这一章整章就是"与 unsafe 的关系"。** 收成一份可以贴在手边的清单：

### 决策流程

```text
我想用 unsafe，因为 ____
   │
   ├─ ① 为了性能
   │     ├─ 先读汇编：安全版本和 unsafe 版本是同一个函数吗？
   │     │     ├─ 是 → 停。你不需要 unsafe。
   │     │     └─ 否 → 继续
   │     ├─ 标准库有没有安全的替代？（split_at_mut / chunks / get_many_mut …）
   │     │     └─ 有 → 用替代。
   │     └─ benchmark：这个差值对整体有影响吗？
   │           └─ 没有 → 停。
   │
   ├─ ② 因为借用检查器不让过
   │     └─ 先问：是我的设计有问题吗？（第 15 章）→ 多半是。
   │
   ├─ ③ FFI / 底层数据结构 / 手写 Send·Sync
   │     └─ 合理。但要走完 26.6 的论证流程。
   │
   └─ ④ 别的库都这么写
         └─ 不是理由。
```

### 写了 `unsafe` 之后的必做项

1. **`SAFETY` 注释**：指出依赖哪条契约、为什么没破坏它（第 24 章）；
2. **前提检查**：能变成 `assert` 的就变成 `assert` —— panic 不是 UB；
3. **`unsafe` 块尽量小**：一块只做一件事；
4. **Miri**：`cargo +nightly miri test`（第 19、25 章）；
5. **把 `unsafe` 收进边界**：对外的接口应该是安全的。

### 三条"看起来对"的写法

| 写法 | 实际代价 | 章 |
|---|---|---|
| 两个 `&mut` 指向同一处 | 破坏 `noalias`，LLVM 已按独占优化 | 24 |
| `addr_of!` 派生自引用指针后写入 | SharedReadOnly tag 被弹掉 | 19 |
| `unsafe impl Send for MyFuture` | 锁会在错误的线程上释放 | 20 |

**三者都不会当场崩、不会报错、可能"碰巧能跑"。**

### 最后一句话

> **`unsafe` 合理的场合，是"你比编译器知道得更多，而且能证明"。**
>
> 而本章的全部内容是：**先确认编译器真的不知道。**
> 因为很多时候，它知道 —— 而且已经替你做了。

## 26.7 小结

- **先读汇编，再谈 `unsafe`。** 它比 benchmark 更快、更确定，
  而且回答的是更早的问题："有没有区别"。
- **`unsafe` 版本经常和安全版本是同一个函数**：
  实测三组对照里有两组是 alias
  （`_sum_unchecked = _sum_safe`、`_masked_unchecked = _masked_safe`）。
- **唯一有差值的情形是"索引来自外部"**：
  `get_safe` 11 条指令 vs `get_unchecked` 2 条 ——
  省下的是 `cmp` + `b.hs` **两条指令**。
- **`unsafe` 可能更慢**：裸指针不带 `noalias`，
  LLVM 必须保守 —— `safe_double_add` 5 条 vs `raw_double_add` 8 条。
  **`unsafe` 不是"解锁更多优化"，而是"放弃优化"。**
- **`&mut` 不比 `&` 慢**：只读场景下两者是**同一个函数**
  （`@sum_shared = ... alias ... ptr @sum_mut_ro`）。
- **先找标准库的安全替代**：`split_at_mut` 解决了
  "劈开切片"和"按索引取两个元素"这两个最经典的 `unsafe` 场景。
- **把 `unsafe` 写小、用 `assert` 把前提变成 panic** ——
  代价恰好就是你想省掉的那两条指令，而换来的是"失败时 panic 而不是 UB"。
- **Miri 通过不等于 sound**：完整的链条是
  汇编 → Miri → `SAFETY` 论证 → benchmark，四步都过才值得。

全书到这里结束。回头看看这条证据链：

```text
源码 → MIR → LLVM IR → 汇编 → 运行时可观察产物
```

每一章都落在其中至少一层上。而贯穿全书的那个主题，
其实是第 24 章那句话的反复变奏：

> **Rust 的安全保证是编译期的，而编译期的信息会变成运行时的代码质量。**
> 类型信息不只是"检查器用来拒绝你的东西"，它是**优化器的输入**。
> `unsafe` 的代价，就是**你替编译器保管这份信息** ——
> 保管得好，什么都不会发生；保管得不好，什么都会发生。
