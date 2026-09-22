# 第 13 章：Arc 与 Mutex 的指令级实现 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> 本机工具链：`rustc 1.98.1` / `cargo 1.98.1` / LLVM 22.1.8。

## 复现命令

```bash
tools/evidence.sh ch13-arc-mutex
scripts/verify-all.sh ch13
```

## 关键结论与断言（3 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | `Arc::clone` 用 `ldadd` 做原子加 | `ldadd` |
| 2 | drop 路径有内存屏障 | `dmb` |
| 3 | 超过引用计数软上限时当前构建会 `brk` | `brk` |

## 原始输出（`.evidence/ch13-arc-mutex-lib.O3.s`，完整函数体）

```asm
	ldr	x8, [x0]         ; 取出 ArcInner 指针
	mov	w9, #1
	ldadd	x9, x9, [x8]     ; ★ 原子加 1（fetch-add），返回旧值
	tbnz	x9, #63, LBB1_4  ; 旧值最高位为 1 → 溢出/极端情况
	sub	sp, sp, #48
	stp	x20, x19, [sp, #16]
	stp	x29, x30, [sp, #32]
	add	x29, sp, #32
	str	x8, [sp, #8]
	ldr	x19, [x8, #16]   ; ★ 读 payload：ArcInner 偏移 16（strong=0, weak=8）
	mov	x9, #-1
	ldaddl	x9, x8, [x8]     ; ★ 原子减 1 —— drop 路径
	cmp	x8, #1
	b.ne	LBB1_3
	dmb	ishld            ; ★ 内存屏障：drop_slow 前的 acquire 语义
	bl	__RNvMsn_..._5alloc4sync...Arc<u64>::drop_slow
LBB1_3:
	mov	x0, x19
	ldp	x29, x30, [sp, #32]
	ldp	x20, x19, [sp, #16]
	add	sp, sp, #48
	ret
LBB1_4:
	brk	#0x1             ; ★ 溢出时真的执行 brk 指令
```

## 讲法要点

- 包装函数展示了 `Arc` 的几条关键实现指令：`ldadd`（加）、`ldaddl`（减）、
  `dmb ishld`（屏障）、`brk`（溢出）。
- 当前 `Arc<u64>` 实例的布局是：`strong` 在 0、`weak` 在 8、payload 在 16；
  这是本次构建的实现观察，不是稳定 ABI。
- **为什么 `Arc<T>: Send` 需要 `T: Send + Sync`**：payload 是被多线程共享的。
- **为什么出现 `brk #0x1`**：超过 `isize::MAX` 软上限后 abort，
  防止计数继续增长并最终回绕。
- **为什么 release 需要 `dmb ishld`**：写入者释放、读者获得。

## Mutex：★ 平台差异（macOS 上是 pthread，不是 futex）

### 实测调用链

`lock_mutex` 的 `-O` AArch64：
```asm
_lock_mutex:
	ldapr	x0, [x0]          ; ★ 读取 pthread Mutex 的 OnceBox 初始化状态
	cbz	x0, LBB6_5
	bl	__RNvMNtNtNtNtNt..._3std3sys3pal4unix4sync5mutexNtB2_5Mutex4lock
	...
	bl	__RNvMNtNtNtNtNt..._3std3sys3pal4unix4sync5mutexNtB2_5Mutex6unlock
```
`try_lock_mutex` 走另一条：
```asm
	bl	__RNvMNtNtNtNtNt..._3std3sys3pal4unix4sync5mutexNtB2_5Mutex8try_lock
	cbz	w0, LBB7_11        ; 失败直接返回，不进等待
```

### 结论

**macOS 上 `Mutex` 的实现是 `pthread_mutex`；Linux 等目标走 futex。**

std 源码（`library/std/src/sys/pal/unix/sync/mutex.rs`）：
```rust
inner: UnsafeCell<libc::pthread_mutex_t>,
...
cvt_nz(libc::pthread_mutexattr_settype(attr, libc::PTHREAD_MUTEX_NORMAL)).unwrap();
cvt_nz(libc::pthread_mutex_init(self.raw(), attr)).unwrap();
let r = unsafe { libc::pthread_mutex_lock(self.raw()) };
```
锁类型被显式设成 **`PTHREAD_MUTEX_NORMAL`** —— 源码注释说明了原因：
这样"同线程重入会死锁"而不是 UB。

⚠️ **"Mutex = futex" 是常见的过度简化。**
- **Windows**：`SRWLock`；
- **macOS 等其他 Unix**：`pthread_mutex`；
- **Linux / Android / FreeBSD 等**：当前 std 走 futex 路径。

因此正文只把 pthread 汇编作为 macOS 实测证据；Linux futex 路径来自
对应工具链的标准库源码，若要引用 Linux 汇编仍需在 Linux 环境验证。

## 断言（6 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | `Arc::clone` 用 `ldadd` 原子加 | `ldadd` |
| 2 | drop 路径有内存屏障 | `dmb` |
| 3 | 超过引用计数软上限时当前构建会 `brk` | `brk` |
| 4 | `Arc::clone` 用的是 **Relaxed**（`ldadd` 而非 `ldaddal`） | 见第 16 章的交叉印证 |
| 5 | `Mutex::lock` 走 `pthread` 实现 | `pal4unix4sync5mutex` |
| 6 | `try_lock` 是非阻塞路径 | `Mutex8try_lock` |

## ★ 补充证据：`MutexGuard` 的 drop 就是 `unlock`

实测：

```bash
grep -c 'unlock' .evidence/ch13-arc-mutex-lib.O3.s     # → 3
grep -o 'Mutex[0-9]*unlock' .evidence/ch13-arc-mutex-lib.O3.s | sort -u
# → Mutex6unlock
```

**这就是第 12 章"`MutexGuard` 是 `!Send`"那条规则的物理原因**：
`MutexGuard::drop` 会调 `unlock`，而 pthread 只允许
**加锁的那个线程**解锁 —— 把 guard 移到别的线程去 drop 是 UB。

## ★ 补充证据：`Arc` 的大小

实测（`size_of`）：

| 类型 | 大小 |
|---|---|
| `Arc<u64>` | **8** |
| `Arc<[u64]>` | **16** |
| `Arc<dyn Fn()>` | **16** |
| `&dyn Fn()` | **16** |
| `ArcInner<u64>`（`2 * usize + u64`） | **24** |

`Arc<u64>` 只有 8 字节 —— 它本体就是**一个指针**，
`strong`/`weak`/`data` 都在堆上的 `ArcInner` 里。
这与 `ldr x8, [x0]`（① 取 `ArcInner` 指针）互相印证。

## ★ `Mutex` 的平台分派（rust-src 原文）

`library/std/src/sys/sync/mutex/mod.rs` 里是一个 `cfg_select!`：

| 平台 | 实现 |
|---|---|
| Linux / Windows / Android / FreeBSD / wasm(atomics) / hermit | `mod futex;` |
| 其他 Unix（**包括 macOS**） | `mod pthread;` |
| Windows 7 | `mod windows7;` |

★ `OnceBox` 属于 pthread 路径
（`sys/sync/mutex/pthread.rs`：`pal: OnceBox<pal::Mutex>`），
因为 `pthread_mutex_t` 需要运行时初始化；futex 路径直接保存原子状态。
这是"抽象泄漏"的教科书案例：同一个 `Mutex` 概念，
在两种平台上的**初始化成本**完全不同。

## 待办

- [ ] 补 channel（第 14 章）的 example
- [ ] 第 14 章：`mpsc` 的发送即 move —— 用 MIR 看所有权转移
- [ ] 如果要加入 futex 汇编证据，需要在 Linux 环境单独验证
