# 13. 线程、`Arc` 与 `Mutex`

> 一句话：上一章说 `Send` / `Sync` 在汇编里**一个字都不剩**。
> 这一章要说反面：`Arc` 的引用计数确实会生成原子指令。
> 下面用一个 clone、读取再 drop 的包装函数，观察 `ldadd`、`ldaddl`、
> `dmb ishld` 与超过引用计数软上限时的 `brk`。

## 先把语法认清

`Arc<T>` 用原子引用计数提供跨线程共享所有权；`Mutex<T>` 用互斥访问把
`&T` 安全地转换成临时的可变访问，锁守卫离开作用域时自动解锁。常见组合
`Arc<Mutex<T>>` 分别回答“谁拥有”与“谁此刻能改”。

一句话记住分工：`Arc` 管**钥匙有几把**，`Mutex` 管**现在谁能进门**。
只有 `Arc` 没有互斥，只有 `Mutex` 又解决不了跨线程共享所有权。

### 放到业务里：共享配置与聚合状态

只读配置可直接用 `Arc<Config>` 分发给 worker；需要复合更新的订单统计、
连接表或内存缓存可以用 `Arc<Mutex<State>>`。关键是缩小 guard 作用域，绝不在
持锁时做慢 IO、用户回调或长计算；简单计数则优先考虑 atomic，避免把所有
worker 串行化在一把锁上。

```rust
let snapshot = Arc::clone(&config);       // 只读共享
let mut stats = stats.lock().unwrap();    // 复合状态短暂独占
stats.record(status, elapsed);
```

两种操作看起来都叫“共享”，一个只增加所有者，一个还建立临界区。

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

## 13.1 先把常见说法摆上桌

通常会这样概括：

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

### 13.2.1 `clone_arc` 包装函数的完整函数体

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

这段函数不只做 `Arc::clone`：它还读取 payload，并在返回前 drop 临时的 `b`。
因此这里同时出现了递增、读取和递减路径。逐条读：

| # | 指令 | 作用 |
|---|---|---|
| ① | `ldr x8, [x0]` | 取出 `ArcInner` 的指针（`Arc` 本体只有一个指针） |
| ② | `ldadd x9, x9, [x8]` | **原子**加 1，返回旧值 |
| ③ | `tbnz x9, #63` | 旧值超过 `isize::MAX` 软上限 → abort 路径 |
| ④ | `ldr x19, [x8, #16]` | 读 payload（偏移 **16**） |
| ⑤ | `ldaddl x9, x8, [x8]` | **原子**减 1（`l` = release 语义） |
| ⑥ | `dmb ishld` | 减到 0 时的屏障（acquire 语义） |
| ⑦ | `brk #0x1` | 超过软上限 → 当前构建内联成 abort trap |

### 13.2.2 `ArcInner` 的内存布局

从 ④ 可以读出**当前工具链下这个 `Arc<u64>` 实例**的布局：payload 在偏移 16。

```text
ArcInner<T> 的布局（AArch64，64 位）：
  偏移 0   : strong  (AtomicUsize)  ← ② 和 ⑤ 操作的就是它
  偏移 8   : weak    (AtomicUsize)
  偏移 16  : data    (T)            ← ④ 读的就是它
```

**为什么是 16？** 因为前面有两个 `AtomicUsize`（各 8 字节）。

> 这是当前 rustc 1.98.1 / AArch64 产物的实现观察，不是 `Arc` 的稳定 ABI。
> `ldr x19, [x8, #16]` 能证明本次构建的偏移，不能约束未来版本。

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

### 13.2.4 为什么超过引用计数软上限会 abort

⑦ 的 `brk #0x1` 是当前 AArch64 构建把 `abort()` 内联后的 trap。

标准库把 `MAX_REFCOUNT` 设为 `isize::MAX`。安全代码也可能通过大量
`Arc::clone` 再 `mem::forget` 人为抬高计数，因此超过阈值不等于内存已经损坏。
但计数若继续增长并最终回绕，就可能过早释放对象，破坏内存安全。
`Arc::clone` 在观察到旧值超过软上限时选择 abort，以阻止程序继续逼近回绕；
源码也明确说明，触发点不保证恰好是 `MAX_REFCOUNT + 1`。

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

`Mutex::lock` 包装函数的 `-O` AArch64 汇编（截取关键部分）：

```asm
_lock_mutex:
	...
	ldapr	x0, [x0]          ; ★ 读取 pthread Mutex 的 OnceBox 初始化状态
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
> Linux 等目标的 futex 路径直接保存原子状态；macOS 所走的 pthread 路径
> 才使用 `OnceBox<pal::Mutex>` 做懒初始化，因为 `pthread_mutex_t` 需要
> 运行时初始化。
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

### 为什么这里选择 abort 而不是 panic

`Arc::clone` 先执行 `fetch_add`，再检查旧值是否超过 `isize::MAX`。
由于 `mem::forget` 可以安全地泄漏 clone 出来的句柄，异常大的计数并不自动
意味着此前已经发生 UB；真正必须阻止的是计数继续增长并最终回绕。

此处不能依赖可恢复的 panic：unwind 后引用计数已经被增加，继续执行会让
这个全局不变量更难维持。当前实现调用 `abort()`，在本次 AArch64 构建里
表现为 `brk #0x1`。这是实现选择，不应把具体 trap 当成语言保证。

### 为什么 `Mutex` 要平台适配

因为**"阻塞"这件事没有可移植的抽象**：

- Linux 有 futex（内核提供的等待队列原语）；
- macOS 只有 pthread；
- Windows 有 SRWLock。

`std` 的解法是定义一层 **`pal`（platform abstraction layer）**，
把"锁"的语义固定下来，实现交给平台。

★ pthread 路径中的 `OnceBox` 暴露了一个**更本质的差异**：
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

### 反直觉之三：包装层也会泄漏平台初始化策略

`lock_mutex` 开头的 `ldapr` + `cbz` 检查的是 pthread mutex 所在
`OnceBox` 是否已经初始化；真正的加锁仍进入 pthread 实现。
不能仅凭这两条指令断言这是锁本身的无竞争 fast path。

这组汇编可靠地说明的是：macOS 的 std 包装层需要懒初始化，Linux 的 futex
实现则直接持有原子状态。至于 pthread 内部如何优化无竞争路径，必须进一步
检查系统库实现或用性能工具测量。

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

# clone + 读 payload + drop 包装函数中的关键指令
awk '/^__RNvCsrtIYgyWToU_3lib9clone_arc:/,/cfi_endproc/' .evidence/ch13-arc-mutex-lib.O3.s

# 只挑关键指令
grep -oE 'ldaddl?|dmb|brk' .evidence/ch13-arc-mutex-lib.O3.s | sort | uniq -c

# Mutex 的平台实现
grep -o 'pal4unix4sync5mutex' .evidence/ch13-arc-mutex-lib.O3.s | head -1
grep -c 'unlock' .evidence/ch13-arc-mutex-lib.O3.s
```

**怎么算验证成功**：

1. `clone_arc` 的函数体里有 `ldadd`（加）、`ldaddl`（减）、
   `dmb ishld`（屏障）、`brk #0x1`（超过软上限后的 trap）四条关键指令；
2. `ldr x19, [x8, #16]` —— 当前构建的 `Arc<u64>` payload 在偏移 16；
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

★ 另一个和 `unsafe` 相关的点是引用计数软上限：标准库必须在计数
可能回绕、进而导致提前释放之前终止进程。这是安全抽象主动防守其
全局不变量的例子。

## 13.7 小结

- **包装函数暴露了 `Arc` 的关键实现痕迹**：`ldadd`（加）、
  `ldaddl`（减）、`dmb ishld`（最后一次 drop 的 acquire fence）以及
  超过引用计数软上限时的 `brk #0x1`。
- **当前 `Arc<u64>` 实例的布局**是 strong=0、weak=8、payload=16；
  这是实现观察，不是稳定 ABI。
- **`Arc::clone` 用的是 `Relaxed`**（`ldadd` 没有 `al` 后缀）——
  引用计数只负责生命周期，不负责数据同步。
  **只有最后一次 drop 才需要 `Release` + `Acquire`。**
- **`Arc<T>: Send + Sync` 要求 `T: Send + Sync`**：
  `Arc` 既让你移动它，也让你共享它 —— 责任最终落在 payload 上。
- **`Arc<u64>` 是 8 字节**（一个指针），但 `Arc<dyn Trait>` 是 16 字节
  （胖指针）—— `Arc` 的胖瘦取决于 `T`。
- **`Mutex` 在 macOS 上是 `pthread_mutex`，Linux 等目标走 futex 实现**。
  `OnceBox` 只属于需要运行时初始化的 pthread 路径；futex 路径直接保存
  原子状态。
- **`MutexGuard` 的 `!Send` 是必需的**：pthread 只允许加锁线程解锁，
  而 `MutexGuard::drop` 就是 `unlock`（实测汇编里 3 次 `unlock`）。
- **`unsafe impl Send/Sync for Arc<T>` 是本章最值得模仿的论证**：
  原子计数 + Release/Acquire 配对 + `T: Send + Sync`，三条缺一不可。

下一章转向**消息传递**：channel 的 `send` 为什么能让编译器
"切断"本地对值的访问，以及 `mpsc` 在 1.98 里的内部实现变了什么。
