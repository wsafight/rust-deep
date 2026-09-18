# 13. 线程、`Arc` 与 `Mutex`

> 一句话：上一章说 `Send` / `Sync` 在汇编里**一个字都不剩**。
> 这一章要说反面：**`Arc` 的线程安全是"真的"有代码的** ——
> 它的全部保证落在四条指令上：`ldadd`（加）、`ldaddl`（减）、
> `dmb ishld`（屏障）、`brk`（溢出）。

## 13.0 一个会让你卡住的例子

`Arc<T>` 是"多线程共享所有权"的标准工具。用起来很简单：

```rust
let a = Arc::new(vec![1u64, 2, 3]);
let b = Arc::clone(&a);           // ← 这里发生了什么？
std::thread::spawn(move || b.len());
println!("{}", a.len());          // ← a 还能用
```

问题：`Arc::clone` **做了什么**？

- 它显然不是"复制一份数据"（那样就不叫共享了）；
- 它显然也不是"什么都不做"（那样两个 `Arc` 怎么知道该何时释放？）；
- 它必须**递增一个计数器**，而且这个计数器要能被多个线程同时递增。

**这个计数器是什么类型？递增它生成什么指令？**
更进一步：`Arc` 是 `Send + Sync` 的，但这是怎么做到的 ——
`Vec<u64>` 和 `Arc<Vec<u64>>` 在线程安全上有什么本质区别？

这一章把 `Arc` 拆到指令级。答案会具体到**四条指令**。

## 13.1 表层解释（官方书会怎么讲）

官方书会说：

- `Arc<T>` = Atomically Reference Counted，原子引用计数；
- `Arc::clone` 递增引用计数，`drop` 递减，减到 0 时释放；
- `Arc` 可以跨线程共享，`Rc` 不行，因为 `Arc` 的计数是**原子**的；
- `Arc<T>: Send + Sync` 要求 `T: Send + Sync`。

这些都对，但"原子的"这三个字里藏着全部信息。
**"原子"在 AArch64 上具体是什么指令？** 官方书不会告诉你。

本章要给的是一条可执行的答案：

```bash
awk '/^__RNvCsrtIYgyWToU_3lib9clone_arc:/,/cfi_endproc/' .evidence/ch13-arc-mutex-lib.O3.s
```

## 13.2 编译器眼里的样子

### 13.2.1 `Arc::clone` 的完整函数体

```asm
__RNvCsrtIYgyWToU_3lib9clone_arc:
	ldr	x8, [x0]         ; ① 取出 ArcInner 指针
	mov	w9, #1
	ldadd	x9, x9, [x8]     ; ② ★ 原子加 1（fetch-add），返回旧值
	tbnz	x9, #63, LBB3_4  ; ③ 旧值最高位为 1 → 溢出路径
	sub	sp, sp, #48
	stp	x20, x19, [sp, #16]
	stp	x29, x30, [sp, #32]
	add	x29, sp, #32
	str	x8, [sp, #8]
	ldr	x19, [x8, #16]   ; ④ ★ 读 payload：ArcInner 偏移 16
	mov	x9, #-1
	ldaddl	x9, x8, [x8]     ; ⑤ ★ 原子减 1 —— 这是 drop 路径
	cmp	x8, #1
	b.ne	LBB3_3
	dmb	ishld            ; ⑥ ★ 内存屏障：drop_slow 前的 acquire 语义
	add	x0, sp, #8
	bl	__RNvMsn_NtCshxvaOLs88l5_5alloc4syncINtB5_3ArcyE9drop_slowCsrtIYgyWToU_3lib
LBB3_3:
	mov	x0, x19
	ldp	x29, x30, [sp, #32]
	ldp	x20, x19, [sp, #16]
	add	sp, sp, #48
	ret
LBB3_4:
	brk	#0x1             ; ⑦ ★ 溢出时真的执行 brk 指令
```

**整个 `Arc` 的线程安全就在这 20 条指令里。** 逐条读：

| # | 指令 | 作用 |
|---|---|---|
| ① | `ldr x8, [x0]` | 取出 `ArcInner` 的指针（`Arc` 本体只有一个指针） |
| ② | `ldadd x9, x9, [x8]` | **原子**加 1，返回旧值 |
| ③ | `tbnz x9, #63` | 旧值最高位为 1 → 计数即将溢出 |
| ④ | `ldr x19, [x8, #16]` | 读 payload（偏移 **16**） |
| ⑤ | `ldaddl x9, x8, [x8]` | **原子**减 1（`l` = release 语义） |
| ⑥ | `dmb ishld` | 减到 0 时的屏障（acquire 语义） |
| ⑦ | `brk #0x1` | 溢出 → 直接 `brk`（不是 panic，是 abort） |

### 13.2.2 `ArcInner` 的内存布局

从 ④ 可以直接读出布局：payload 在偏移 **16**。

```text
ArcInner<T> 的布局（AArch64，64 位）：
  偏移 0   : strong  (AtomicUsize)  ← ② 和 ⑤ 操作的就是它
  偏移 8   : weak    (AtomicUsize)
  偏移 16  : data    (T)            ← ④ 读的就是它
```

**为什么是 16？** 因为前面有两个 `AtomicUsize`（各 8 字节）。

> 这个偏移不是"文档说的"，是**实测出来的** ——
> `ldr x19, [x8, #16]` 里的 `#16` 就是证据。

### 13.2.3 `ldadd` 而不是 `ldaddal` —— 一个重要的细节

② 用的是 **`ldadd`**（不是 `ldaddal`），⑤ 用的是 **`ldaddl`**。

AArch64 的后缀是有含义的：

| 指令 | 语义 |
|---|---|
| `ldadd` | 纯原子，**无**内存序 |
| `ldaddl` | 原子 + **release**（`l` = release） |
| `ldadda` | 原子 + **acquire** |
| `ldaddal` | 原子 + **acquire-release** |

**为什么 clone 用无内存序的 `ldadd`？**

因为**递增引用计数本身不需要任何顺序保证**。
它只是在说"又多了一个持有者"—— 多一个持有者这件事，
不需要对任何内存的可见性做出承诺。

**但最后一次 drop 需要。** ⑤ 的 `ldaddl`（release）+ ⑥ 的 `dmb ishld`（acquire）
合起来保证了：**销毁 `ArcInner` 的那个线程，能看到之前所有持有者对 payload 的写入。**

> **这是 `Arc` 健全性论证的核心**：
> 引用计数的增减是 `Relaxed` 的，**只有最后一次 drop 是 `Release` + `Acquire`**。
> 这就是为什么 `Arc::clone` 的 `ldadd` 没有后缀。

★ 这个结论可以和**第 16 章**交叉验证：

```text
ch13: ldadd    ← Arc::clone  → 对应 Ordering::Relaxed
ch16: ldadd    ← fetch_add(Relaxed)
      ldaddal  ← fetch_add(SeqCst)
```

**两条独立路径得到同一个结论** —— 一个从 `Arc` 的实现读出来，
一个从 `fetch_add` 的不同 `Ordering` 读出来。

### 13.2.4 溢出为什么是 `brk` 而不是 `panic!`

⑦ 的 `brk #0x1` 是 AArch64 的"断点"指令，直接触发 abort。

**为什么不用 `panic!`？** 因为引用计数溢出意味着**内存已经被破坏了** ——
此时再去做 panic（要格式化字符串、要 unwinding、要分配内存）
可能踩到已经损坏的状态。**直接 abort 是唯一安全的选择。**

实测确认：这条 `brk` **真的**生成了，不是文档传说。

### 13.2.5 `Arc<T>: Send + Sync` 的条件

第 12 章讲过 `Arc<T>: Send` 要求 `T: Send + Sync`。现在可以解释**为什么**：

- `Arc` 让你**移动**它到另一个线程 → 需要 `T: Send`；
- `Arc` 让你**共享**它（多个线程同时持有 `Arc<T>`，都能拿到 `&T`）→ 需要 `T: Sync`。

**payload 是被多线程共享的**，而 `Arc` 本身只是一个指针 + 计数。
所以线程安全的真正责任在 `T` 上 —— `Arc` 只负责让"共享"这件事有正确的生命周期。

对照第 12 章的实测：

```text
error[E0277]: `Cell<u64>` cannot be shared between threads safely
   = help: the trait `Sync` is not implemented for `Cell<u64>`
   = note: required for `Arc<Cell<u64>>` to implement `Send`
```

### 13.2.6 `Mutex`：★ 平台差异（macOS 上是 pthread，不是 futex）

`Mutex::lock` 的 `-O` AArch64 汇编（截取关键部分）：

```asm
_lock_mutex:
	...
	ldapr	x0, [x0]          ; ★ 先做一次 acquire 读（fast path 尝试）
	cbz	x0, LBB6_5
	bl	__RNvMNtNtNtNtNtCs82bWklYMk3w_3std3sys3pal4unix4sync5mutexNtB2_5Mutex4lock
	...
```

`try_lock` 走另一条：

```asm
_try_lock_mutex:
	...
	ldapr	x0, [x0]
	cbz	x0, LBB7_10
	bl	__RNvMNtNtNtNtNtCs82bWklYMk3w_3std3sys3pal4unix4sync5mutexNtB2_5Mutex8try_lock
	cbz	w0, LBB7_11        ; 失败直接返回，不进等待
```

符号名里的 **`pal4unix4sync5mutex`** 就是答案：
`std::sys::pal::unix::sync::mutex` —— **pthread 实现**。

`rust-src` 里的实际代码（`library/std/src/sys/pal/unix/sync/mutex.rs`）：

```rust
pub struct Mutex {
    inner: UnsafeCell<libc::pthread_mutex_t>,
}

pub unsafe fn lock(self: Pin<&Self>) {
    let r = unsafe { libc::pthread_mutex_lock(self.raw()) };
    ...
}
```

★ 源码里还有一段注释解释了锁类型的选择：

```rust
// A pthread mutex initialized with PTHREAD_MUTEX_INITIALIZER will have
// a type of PTHREAD_MUTEX_DEFAULT, which has undefined behavior if you
// try to re-lock it from the same thread when you already hold a lock.
...
// Since locking the same mutex twice will result in two aliasing &mut
// references, we instead create the mutex with type
// PTHREAD_MUTEX_NORMAL which is guaranteed to deadlock if we try to
// re-lock it from the same thread, thus avoiding undefined behavior.
cvt_nz(libc::pthread_mutexattr_settype(attr, libc::PTHREAD_MUTEX_NORMAL)).unwrap();
```

**"同线程重入 → 死锁"是被**故意**选的**，因为死锁比 UB 好。

> ⚠️ **"`Mutex` = futex" 是一个常见的过度简化。**
> 实测在 macOS 上完全复现不出来。
>
> 而且这里还有一层：`sys::sync::mutex` 在**不同平台走不同实现** ——
> `rust-src` 的 `std/src/sys/sync/mutex/mod.rs` 里是一个 `cfg_select!`：
>
> | 平台 | 实现 |
> |---|---|
> | Linux / Windows / Android / FreeBSD… | `futex`（`mod futex;`） |
> | 其他 Unix（**包括 macOS**） | `pthread`（`mod pthread;`） |
>
> **`futex` 路径在 Linux 上才存在** —— 但即使在那里，
> `sys::Mutex` 也是包在一个 `OnceBox` 里的
> （`sys/sync/mutex/pthread.rs`：`pal: OnceBox<pal::Mutex>`），
> 因为 `pthread_mutex_t` 需要运行时初始化。
>
> **所以正确说法是**：`std::sync::Mutex` 是一个**平台适配的包装**，
> 它的语义（阻塞、`PTHREAD_MUTEX_NORMAL` 的死锁行为）由 `pal` 层保证。
> 想讲 futex 需要**在 Linux 上验证** —— 这不是本书本机的复现路径。

★ 顺带一个和上一章呼应的事实：**`MutexGuard` 的 drop 会调用 `unlock`**。
实测 `grep -c unlock` 在 `.O3.s` 里命中 **3** 次，
符号是 `...Mutex6unlock`。这就是第 12 章"`MutexGuard` 是 `!Send`"
那条规则的**物理原因**：解锁必须发生在加锁的那个线程。

## 13.3 为什么必须这样设计

### 为什么引用计数用 `Relaxed` 就够

这是 `Arc` 最容易被误解的地方。直觉上"多线程共享的计数器"应该用最强的
`SeqCst`，为什么 `Arc::clone` 用的是无内存序的 `ldadd`？

**因为引用计数不是用来同步数据的，它是用来决定"什么时候可以销毁"的。**

- `clone` 时：多一个持有者。这个事实**不需要**对任何内存可见性做出承诺；
- `drop` 时：少一个持有者。如果减到 0，**我**负责销毁 ——
  此时我必须能看到**其他所有持有者**对 payload 的写入。

所以顺序要求**只出现在最后一步**：

```text
clone:  ldadd         （Relaxed）
drop:   ldaddl        （Release）
        ↓ 如果减到 0
        dmb ishld     （Acquire）
        然后销毁
```

**Release-Acquire 配对**保证了：销毁线程能看到所有之前的写。

> 这个论证可以在 `std` 源码里逐字读到
> （`library/alloc/src/sync.rs` 里 `Arc` 的 `Drop` 实现附近）。

### 为什么溢出要 `brk` 而不是 panic

引用计数溢出（`usize` 用满）意味着程序已经持有了 2^64 个引用 ——
这在现实中只可能发生在**内存已经被破坏**的情况下。

此时：

- `panic!` 要格式化消息 → 要分配内存 → 可能再次触发损坏；
- unwinding 要遍历栈 → 可能碰到已经释放的帧。

**`brk` 是唯一不依赖任何运行时设施的选择。** 这是"防御性编程"的极端形式。

### 为什么 `Mutex` 要平台适配

因为**"阻塞"这件事没有可移植的抽象**：

- Linux 有 futex（内核提供的等待队列原语）；
- macOS 只有 pthread；
- Windows 有 SRWLock。

`std` 的解法是定义一层 **`pal`（platform abstraction layer）**，
把"锁"的语义固定下来，实现交给平台。

★ 而 `OnceBox` 那一层暴露了一个**更本质的差异**：
pthread 的 `pthread_mutex_t` **需要运行时初始化**
（`pthread_mutex_init` 要设置 `attr`），而 futex 只需要一个 `AtomicU32`。
所以 `sys::sync::mutex::Mutex` 在 pthread 路径上必须多一层 `OnceBox`
（一个懒初始化的 `Box<pal::Mutex>`）。

**这是"抽象泄漏"的教科书案例**：同一个 `Mutex` 概念，
在两种平台上的**初始化成本**完全不同。

## 13.4 反直觉的点

### 反直觉之一：`Arc::clone` 是**无内存序**的

很多人以为"原子操作越强越好"，会以为 `Arc::clone` 用的是 `SeqCst`。

**实测：`ldadd`（没有任何后缀）= `Relaxed`。**

原因见 13.3：引用计数只负责"生命周期"，不负责"数据同步"。
**用更强的内存序在这里是纯粹的浪费**（在 AArch64 上，
`ldaddal` 比 `ldadd` 需要额外的屏障指令）。

> 这条也是"内存序不是越强越好"的最佳例证 ——
> 第 16 章会展开讲每一种 `Ordering` 的适用场景。

### 反直觉之二：`Arc` 本身只是**一个指针**

`Arc<u64>` 的大小是 **8 字节**（一个指针），不是 16 或 24。

因为 `strong` / `weak` / `data` 都在**堆上**的 `ArcInner` 里，
`Arc` 本体只有一个指向它的指针。这从 ① 的 `ldr x8, [x0]` 可以直接读出来：
**多一层间接**换来的是"所有 `Arc` 副本共享同一个计数"。

实测（`size_of`）：

| 类型 | 大小 |
|---|---|
| `Arc<u64>` | **8** |
| `Arc<[u64]>` | **16** |
| `Arc<dyn Fn()>` | **16** |
| `&dyn Fn()` | **16** |
| `ArcInner<u64>`（`2 * usize + u64`） | **24** |

★ 对比 `&dyn Trait`（16 字节的胖指针，第 7 章）：
`Arc<dyn Trait>` 是 **16 字节**（数据指针 + vtable 指针），
但 `Arc<u64>` 是 8 字节。**`Arc` 的胖瘦取决于 `T`** ——
因为 `Arc<T>` 里存的是 `*const ArcInner<T>`，
而 `ArcInner<T>` 的大小只影响**堆上**的分配。

### 反直觉之三：`Mutex` 的"快路径"也在汇编里

`lock_mutex` 的第一条是 `ldapr x0, [x0]` ——
一次 **acquire 读**，然后 `cbz` 判断。

这是 std 的 fast path：**先试着直接拿锁**，
拿不到才进 `pthread_mutex_lock`（那里会做真正的阻塞等待）。

**所以"加锁"不是一个原子操作，是"一次原子读 + 可能的系统调用"。**
这解释了为什么无竞争时 `Mutex` 很便宜，有竞争时会突然变贵。

### 反直觉之四：`MutexGuard` 的 `!Send` 不是"保守"，是**必需**

第 12 章说过 `MutexGuard` 是 `!Send`。现在能看到它的物理原因：

实测 `unlock` 在汇编里出现 **3** 次（符号 `...Mutex6unlock`）——
**`MutexGuard` 的 `Drop` 就是要调 `unlock`**。

而 pthread 的规则是"**只有加锁的线程能解锁**"。
把 guard 移到别的线程去 drop，就是在别的线程 unlock —— **未定义行为**。

**所以 `!Send` 不是设计者的谨慎，是 pthread 语义的直接映射。**

## 13.5 亲手验证

```bash
tools/evidence.sh ch13-arc-mutex
scripts/verify-all.sh ch13

# ★ 全部四条指令（完整函数体）
awk '/^__RNvCsrtIYgyWToU_3lib9clone_arc:/,/cfi_endproc/' .evidence/ch13-arc-mutex-lib.O3.s

# 只挑关键指令
grep -oE 'ldaddl?|dmb|brk' .evidence/ch13-arc-mutex-lib.O3.s | sort | uniq -c

# Mutex 的平台实现
grep -o 'pal4unix4sync5mutex' .evidence/ch13-arc-mutex-lib.O3.s | head -1
grep -c 'unlock' .evidence/ch13-arc-mutex-lib.O3.s
```

**怎么算验证成功**：

1. `clone_arc` 的函数体里有 `ldadd`（加）、`ldaddl`（减）、
   `dmb ishld`（屏障）、`brk #0x1`（溢出）四条关键指令；
2. `ldr x19, [x8, #16]` —— **payload 在偏移 16**（前面两个 `AtomicUsize`）；
3. `ldadd` **没有** `al` 后缀 —— `Arc::clone` 用的是 `Relaxed`；
4. `lock_mutex` 的符号里出现 `pal4unix4sync5mutex` —— **macOS 上是 pthread**；
5. `grep -c unlock` 命中 3 次 —— `MutexGuard::drop` 会解锁。

```bash
scripts/verify-all.sh ch13      # 5 条断言
```

## 13.6 与 unsafe 的关系

这一章是"**`unsafe` 实现了什么**"的最佳样本 ——
`Arc` 的内部几乎全是 `unsafe`，但它提供的接口**完全安全**。

★ 值得逐条对照的是 `Arc` 里那两个 `unsafe impl`
（`rust-src` 的 `alloc/src/sync.rs`，**原文**）：

```rust
unsafe impl<T: ?Sized + Sync + Send, A: Allocator + Send> Send for Arc<T, A> {}
unsafe impl<T: ?Sized + Sync + Send, A: Allocator + Sync> Sync for Arc<T, A> {}
```

（注意 `ArcInner<T>` 也有对应的一对，第 412–413 行。）

**这两个 impl 就是"撒谎"** —— 编译器不会验证 `Arc` 真的线程安全。
但它撒得**有根据**，论证是：

1. `strong` / `weak` 是 `AtomicUsize`，增减都是原子的（`ldadd` / `ldaddl`）；
2. **只有**减到 0 的那一次会销毁，而那次有 `Release` + `Acquire` 配对
   （`ldaddl` + `dmb ishld`），所以销毁线程能看到所有先前的写；
3. `T: Send + Sync` 保证了 payload 本身可以安全地被移动和共享。

**三条合起来，就是 `unsafe impl Send/Sync` 的完整论证。**
第 24–25 章会给出更多这样的论证模板。

★ 另一个和 `unsafe` 相关的点是 `brk #0x1`：
**标准库宁愿直接 abort，也不愿在可能已损坏的状态下做 panic。**
这是"防御性设计"在 `unsafe` 边界上的体现。

## 13.7 小结

- **`Arc` 的线程安全落在四条指令上**：
  `ldadd`（加）、`ldaddl`（减）、`dmb ishld`（屏障）、`brk #0x1`（溢出）。
- **`ArcInner` 的布局**：`strong` 在 0、`weak` 在 8、**payload 在 16**
  （`ldr x19, [x8, #16]` 就是证据）。
- **`Arc::clone` 用的是 `Relaxed`**（`ldadd` 没有 `al` 后缀）——
  引用计数只负责生命周期，不负责数据同步。
  **只有最后一次 drop 才需要 `Release` + `Acquire`。**
- **`Arc<T>: Send + Sync` 要求 `T: Send + Sync`**：
  `Arc` 既让你移动它，也让你共享它 —— 责任最终落在 payload 上。
- **`Arc<u64>` 是 8 字节**（一个指针），但 `Arc<dyn Trait>` 是 16 字节
  （胖指针）—— `Arc` 的胖瘦取决于 `T`。
- **`Mutex` 在 macOS 上是 `pthread_mutex`，不是 futex**：
  符号名 `pal4unix4sync5mutex` 就是证据；`futex` 路径只在 Linux 上存在，
  且仍然包在 `OnceBox` 里（`pthread_mutex_t` 需要运行时初始化）。
- **`MutexGuard` 的 `!Send` 是必需的**：pthread 只允许加锁线程解锁，
  而 `MutexGuard::drop` 就是 `unlock`（实测汇编里 3 次 `unlock`）。
- **`unsafe impl Send/Sync for Arc<T>` 是本章最值得模仿的论证**：
  原子计数 + Release/Acquire 配对 + `T: Send + Sync`，三条缺一不可。

下一章转向**消息传递**：channel 的 `send` 为什么能让编译器
"切断"本地对值的访问，以及 `mpsc` 在 1.98 里的内部实现变了什么。
