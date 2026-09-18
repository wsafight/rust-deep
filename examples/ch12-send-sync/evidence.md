# 第 12 章：Send 与 Sync 的真相 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch12-send-sync
scripts/verify-all.sh ch12       # 9 条断言（PASS=11）
```

## 关键结论与断言（9 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | `Rc<T>` 不能跨线程（`!Send`） | `fail/not_send.rs` → **E0277** |
| 2 | `Cell<T>` 不能共享引用（`!Sync`） | `fail/not_sync.rs` → **E0277** |
| 3 | `unsafe impl Send` 之后 `MyBox` 可以跨线程 | `.O3.s` 的 `^_spawn_mybox:` |
| 4 | `Cell<u64>` **是** `Send`（只是不是 `Sync`） | `.O3.s` 的 `^_cell_is_send:` |
| 5 | `PhantomData<*const T>` 让类型 `!Send` | `fail/phantom_not_send.rs` → 报出 `PhantomData<*const u64>` |
| 6 | `PhantomData<fn() -> T>` 版本是 `Send` | `.O3.s` 的 `^_spawn_safe_handle:` |
| 7 | 精确捕获：只捕获 `id` 就能过 | `.O3.s` 的 `^_spawn_handle_field:` |
| 8 | `MutexGuard` **是** `Sync` | `.O3.s` 的 `^_guard_is_sync:` |
| 9 | `MutexGuard` **不是** `Send` | `fail/guard_not_send.rs` → **E0277** |

## ★ 核心结论：汇编里**没有**任何"检查 `Send`"的代码

实测：

```bash
grep -c 'Send\|Sync' .evidence/ch12-send-sync-lib.O3.s
# → 18
```

18 处命中**全部**在 mangled 符号名内部，例如：

```text
__RINvNtCs8Mbv00yxnRz_4core3ptr9drop_glueINtNtB4_4cell10UnsafeCellINtNtB4_6option6OptionINtNtB4_6result6ResultjINtNtCshxvaOLs88l5_5alloc5boxed3BoxDNtNtB4_3any3AnyNtNtB4_6marker4SendEL_EEEEECsrtIYgyWToU_3lib:
```

把 `SendEL_` 解出来是 `Box<dyn Any + Send>` ——
这是 **std 内部 panic 机制**的 drop glue 里的类型名，
和本章的代码无关。

`grep -o 'Send[A-Za-z0-9_]*'` 的完整去重结果：

```text
SendEL_EECsrtIYgyWToU_3lib
SendEL_EEECsrtIYgyWToU_3lib
SendEL_EEEEECsrtIYgyWToU_3lib
SendEL_ENtNtBM_3fmt5Debug3fmtCsrtIYgyWToU_3lib
SendEL_NtNtB7_3fmt5Debug3fmt
```

**全部以 `SendEL_` 结尾**（`EL_` 是 `+ Send` 的编码），
**没有任何一处是"检查这个值能不能跨线程"的指令**。

### 对照：单线程里 `Cell` / `Rc` 完全没有额外开销

`local_only`（同时用 `Cell` 和 `Rc`）的完整函数体：

```asm
_local_only:
	ldr	x8, [x0]        ; c.get()
	add	x8, x8, #1
	str	x8, [x0]        ; c.set()
	ldr	x9, [x1]        ; r 的 ArcInner 指针
	ldr	x9, [x9, #16]   ; payload（偏移 16，与第 13 章一致）
	add	x0, x9, x8
	ret
```

**8 条指令**，没有一条和线程安全有关。
`Cell` 的"非同步"、`Rc` 的"非原子引用计数"在这里都**不产生任何代码** ——
因为这个函数只在单线程里用，而这件事已经由类型系统保证了。

## ★ 反证：不满足 `Send` 的程序**根本没有汇编**

```bash
$ rustc --edition 2024 --crate-type=lib examples/ch12-send-sync/fail/not_send.rs
error[E0277]: `Rc<u64>` cannot be sent between threads safely
   --> examples/ch12-send-sync/fail/not_send.rs:7:24
    |
  7 |     std::thread::spawn(move || {
    |                       ^^^^^^^^ `Rc<u64>` cannot be sent between threads safely
```

**检查在编译期，不在运行期。** 这就是"零成本抽象"在并发上的体现。

## ★★ `Sync` 的定义：`&T: Send`

`fail/not_sync.rs` 的完整输出里有一行**关键** note：

```text
error[E0277]: `Cell<u64>` cannot be shared between threads safely
  --> examples/ch12-send-sync/fail/not_sync.rs:29:24
   |
29 |     std::thread::spawn(move || c.set(1));
   |     ------------------ ^^^^^^^^^^^^^^^^ `Cell<u64>` cannot be shared between threads safely
   |
   = help: the trait `Sync` is not implemented for `Cell<u64>`
   = note: if you want to do aliasing and mutation between multiple threads, use `std::sync::RwLock` or `std::sync::atomic::AtomicU64` instead
   = note: required for `&Cell<u64>` to implement `Send`
```

★ **最后一行**：编译器要检查的不是 `Cell<u64>`，而是 **`&Cell<u64>`**。
标准库里的实际 impl 就是：

```rust
impl<T: ?Sized + Sync> Send for &T {}
```

**`Sync` 就是"`&T` 是 `Send`"这件事的名字。**

| | 含义 | 检查的是 |
|---|---|---|
| `T: Send` | 值本身能**搬**过去 | `T` |
| `T: Sync` | **共享引用**能搬过去 | `&T` |

### `Cell<u64>` 是 `Send` 但 `!Sync`（不对称）

| 写法 | 结果 |
|---|---|
| `thread::spawn(move \|\| c.get())`（`c: Cell<u64>`） | ✅ 编译通过 |
| `thread::spawn(move \|\| r.set(1))`（`r: &Cell<u64>`） | ❌ E0277 |

`src/lib.rs` 的 `cell_is_send` 就是第一行的证据。
**`!Sync` 不是"这个类型有毒"，是"它只在一个线程里安全"。**

## ★★ 实测踩到的坑：`unsafe impl Send` 只管**它标在哪个类型上**

这是本章最值钱的一段。

```rust
pub struct MyBox(pub *mut u64);
unsafe impl Send for MyBox {}      // ← 标在 MyBox 上

pub fn spawn_mybox(b: MyBox) -> usize {
    let h = std::thread::spawn(move || b.0 as usize);   // ← 编译不过！
    h.join().unwrap()
}
```

**报错**：

```text
error[E0277]: `*mut u64` cannot be sent between threads safely
   = help: within `{closure@...}`, the trait `Send` is not implemented for `*mut u64`
note: required because it's used within this closure
```

**原因**：edition 2021 起闭包做**精确捕获**（disjoint capture），
`move || b.0 as usize` 只捕获字段 `b.0`（类型 `*mut u64`），
**不是整个 `MyBox`** —— 于是 `unsafe impl Send for MyBox` **根本没被用上**。

**修法**：强制捕获整个结构体

```rust
let h = std::thread::spawn(move || {
    let _ = &b;        // ← 强制捕获整个 MyBox
    b.0 as usize
});
```

**深水洞察**：
- **`Send` 是"某个类型"的标记，不是"某段内存"的标记。**
- 标记加在 `MyBox` 上，就**只管** `MyBox` 本身。
- 这是精确捕获（edition 2021+）的一个**副作用**：
  捕获粒度变细之后，**类型层的标记也跟着变细了**。

## ★ `PhantomData` 决定 auto trait

```rust
pub struct Handle<T> { id: u64, _p: PhantomData<*const T> }
```

实测错误信息（`fail/phantom_not_send.rs`）：

```text
error[E0277]: `*const u64` cannot be sent between threads safely
   = help: within `{closure@...}`, the trait `Send` is not implemented for `*const u64`
note: required because it appears within the type `PhantomData<*const u64>`
   --> .../core/src/marker.rs:811:12
    |
811 | pub struct PhantomData<T: PointeeSized>;
```

★ **最后一行的措辞值得逐字读**：编译器指的是 `PhantomData<*const u64>`，
**不是 `Handle<u64>`** —— 它直接把"谁该为 `!Send` 负责"点了出来。

| `PhantomData<...>` | `Send`? | `Sync`? |
|---|---|---|
| `PhantomData<*const T>` | ❌ | ❌ |
| `PhantomData<fn() -> T>` | ✅ | ✅ |
| `PhantomData<T>` | 跟随 `T` | 跟随 `T` |

**因为 `fn() -> T` 是函数指针，而函数指针总是 `Send + Sync`。**

实测对照：`SafeHandle<T>`（`PhantomData<fn() -> T>`）的 `spawn_safe_handle`
编译通过；`Handle<T>` 不行 —— **同一个 `T`，只差 `PhantomData` 的写法**。

### 精确捕获的**正面**用法

同一个机制在 `Handle` 上反而是好事：

| 写法 | 捕获了什么 | 结果 |
|---|---|---|
| `move \|\| h.id` | 只有 `id: u64` | ✅（`^_spawn_handle_field:`） |
| `move \|\| { let _ = &h; h.id }` | 整个 `Handle<u64>` | ❌ E0277 |

**捕获粒度决定了哪个类型被拿去检查 auto trait** ——
这既是坑（`unsafe impl Send` 被绕过），也是工具。

## ★★ `Sync` 与 `Send` 是两个**独立**的性质

`fail/guard_not_send.rs` 补上了 12.0 那张表的最后两个格子：

| 类型 | `Send` | `Sync` | 证据 |
|---|---|---|---|
| `Cell<u64>` | ✅ | ❌ | `src/lib.rs::cell_is_send` / `fail/not_sync.rs` |
| `MutexGuard<'_, u64>` | ❌ | ✅ | `fail/guard_not_send.rs` / `src/lib.rs::guard_is_sync` |
| `Rc<u64>` | ❌ | ❌ | `fail/not_send.rs` |
| `u64` | ✅ | ✅ | — |

**四个格子全部填满 → 两个 trait 之间没有任何蕴含关系。**

实测（`fail/guard_not_send.rs`）：

```text
error[E0277]: `std::sync::MutexGuard<'_, u64>` cannot be sent between threads safely
   --> examples/ch12-send-sync/fail/guard_not_send.rs:33:24
    |
 33 |       std::thread::spawn(move || {
    |       ------------------ ^------
 35 |     });
    | |_____^ `std::sync::MutexGuard<'_, u64>` cannot be sent between threads safely
```

★ **为什么 `MutexGuard` 是 `!Send`？**
因为 pthread 只保证"**加锁的那个线程**能解锁"。
把 guard 移到别的线程去 drop（= unlock）是未定义行为。
标准库因此**只**给它实现了 `Sync`，**没有**实现 `Send`。

★ 而 `guard_is_sync`（`&MutexGuard` 跨线程）**编译通过** ——
因为共享引用不解锁。

★ 这解释了 `Arc<T>: Send` 为什么要求 `T: Send + Sync`：
`Arc` 既让你**移动**它（要 `Send`），也让你**共享**它（要 `Sync`）。
实测（`Arc<Cell<u64>>`）：

```text
error[E0277]: `Cell<u64>` cannot be shared between threads safely
   = help: the trait `Sync` is not implemented for `Cell<u64>`
   = note: required for `Arc<Cell<u64>>` to implement `Send`
```

## 交叉验证（可选）

```bash
# 命中的全在符号名里
grep -o 'Send[A-Za-z0-9_]*' .evidence/ch12-send-sync-lib.O3.s | sort -u

# 单线程零开销
awk '/^_local_only:/,/cfi_endproc/' .evidence/ch12-send-sync-lib.O3.s
```
