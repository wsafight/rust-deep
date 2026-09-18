# 第 19 章：`Pin` 与 `Unpin` 为什么存在 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> Miri：`miri 0.1.0 (923c95cdf5 2026-09-16)`（`rustc 1.100.0-nightly`）

## 复现命令

```bash
tools/evidence.sh ch19-pin
scripts/verify-all.sh ch19      # 13 条断言（PASS=15）

# ★ Miri（需要 nightly，与 verify-all.sh 分开）
scripts/verify-miri.sh          # 5 条：ch25 两条 + ch19 两条 + 前置检查
```

## 关键结论与断言（13 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | `Pin<&mut T>` 与 `&mut T` 生成同一个函数 | `.O3.ll` 里 `@plain = ... alias ... ptr @pinned` |
| 2 | 汇编里两者是同一个符号 | `.O3.s` 里 `^_plain = _pinned` |
| 3 | `u64` 与普通结构体都是 `Unpin` | `.O3.ll` 里 `@pin_a_u64 = ... alias ... ptr @pin_a_plain` |
| 4 | `PhantomPinned` 让类型 `!Unpin` | `fail/pin_requires_unpin.rs` → **E0277** `PhantomPinned cannot be unpinned` |
| 5 | 拿不到 `&mut T` | `fail/move_pinned.rs` → **E0596** |
| 6 | 也移不走 | `fail/move_pinned.rs` → **E0507** |
| 7 | ★ 自引用 = 一条 `stp` | `.O3.s` 里 `stp x19, x0, [x0]` |
| 8 | `Box::pin` 真的走堆分配 | `.O3.s` 里 `bl ___rust_alloc` |
| 9 | 读自引用要两次解引用 | `.O3.s` 里 `ldr x8, [x8, #8]` |
| 10 | 拿 `&mut` 必须走 `unsafe` | `.mir` 里 `get_unchecked_mut` |
| 11 | ★ 状态机里一个字段指向另一个字段 | `.mir` 里 `field _s1: &String` |
| 12 | `Suspend0` 变体同时装着被借者和借用者 | `.mir` 里 `Suspend0 (3): [_s0, _s1, _s2]` |
| 13 | ★ 所有 `async` 产物都是 `!Unpin` | `fail/future_not_unpin.rs` → `cannot be unpinned` |

## ★ 核心证据一：自引用的运行时形态（一条 `stp`）

```rust
pub struct SelfRef {
    pub data: u64,
    self_ref: *mut u64,     // 指向 self.data
    _pin: PhantomPinned,
}

pub fn make_self_ref(data: u64) -> Pin<Box<SelfRef>> {
    let mut b = Box::pin(SelfRef { data: 0, self_ref: std::ptr::null_mut(), _pin: PhantomPinned });
    let this: &mut SelfRef = unsafe { b.as_mut().get_unchecked_mut() };
    this.data = data;
    this.self_ref = std::ptr::addr_of_mut!(this.data);
    b
}
```

O3 汇编（`aarch64-apple-darwin` / `rustc 1.98.1` / `-O` / `#[unsafe(no_mangle)]`）：

```asm
_make_self_ref:
	mov	x19, x0              ; 保住参数 data
	mov	w0, #16              ; 16 字节 = u64(8) + 指针(8)
	mov	w1, #8               ; align 8
	bl	___rust_alloc_zeroed ; ← 堆分配：地址稳定的来源
	cbz	x0, LBB1_2           ; 分配失败 → panic
	stp	x19, x0, [x0]        ; ★ data 和"指向自己的地址"一起写进去
	ret
```

**`stp x19, x0, [x0]` 就是自引用的全部运行时形态**：

| 写入 | 含义 |
|---|---|
| `[x0] = x19` | `data` 字段 = 传入的 data |
| `[x0 + 8] = x0` | `self_ref` 字段 = **分配到的地址本身** |

**没有运行时登记，就是"把自己的地址存进自己"。**

对照 O0（两条 `str` 分开）：

```asm
	ldr	x9, [sp, #8]     ; this: &mut SelfRef
	ldr	x8, [sp, #24]    ; data
	str	x8, [x9]         ; this.data = data
	mov	x8, x9
	str	x8, [x9, #8]     ; this.self_ref = &this.data
```

O3 把两条合并成一条 `stp` —— 这就是"自引用"和"两个普通字段"
在机器码层面没有区别的原因。

读回来（`read_self_ref`）：

```asm
_read_self_ref:
	ldr	x8, [x0]        ; 从 &Pin<Box<SelfRef>> 取出 Box 指针
	ldr	x8, [x8, #8]    ; 取 self_ref 字段（偏移 8）
	ldr	x0, [x8]        ; 顺着它读 data
	ret
```

⚠️ 符号名里的 hash（`CsrtIYgyWToU`）**每次编译都会变**，不要当常量。

## ★ 核心证据二：`async` 状态机真的自引用

`borrow_across_await()` 的 MIR（`examples/ch19-pin/src/lib.rs`）：

```mir
coroutine layout {
    field _s0: String;                  // ← 被借用的那个
    field _s1: &String;                 // ← 借用的那个（自引用！）
    field _s2: std::future::Ready<()>;
    variant_fields = {
        Unresumed(0): [],
        Returned (1): [],
        Panicked (2): [],
        Suspend0 (3): [_s0, _s1, _s2],  // ← 挂起时三个字段同时活着
    }
    storage_conflicts = BitMatrix(3x3) { ... }
}
```

★ **`_s1: &String` 就是手写版本里的 `self_ref`。**
第 18 章说"状态机每个 `await` 一个变体"；这里看到的是同一件事的另一面：
**一个变体里的两个字段互相指涉。这就是编译器必须保守的原因。**

## ★ 核心证据三：Miri —— `addr_of!` vs `addr_of_mut!`

这是本章最反直觉、也是最有价值的一条。

```rust
this.self_ref = std::ptr::addr_of!(this.data);      // A
this.self_ref = std::ptr::addr_of_mut!(this.data);  // B
```

**两者的汇编完全一样**（都是取 `this + 0` 的地址）。但 Miri 判定相反：

| 写法 | Miri（Stacked Borrows） |
|---|---|
| A `addr_of!` | ❌ `Undefined Behavior: attempting a read access using <N> at alloc<N>[0x0], but that tag does not exist in the borrow stack for this location` |
| B `addr_of_mut!` | ✅ 通过 |

**为什么**：指针的**出处**决定它的**权限**。

- `addr_of!(place)` → **SharedReadOnly** —— 之后任何一次写都会把它**弹掉**；
- `addr_of_mut!(place)` → **Unique** —— 写入不会作废它。

自引用结构**必然**要先写 `data`、再写 `self_ref`，所以只有 B 是 sound 的。

★ **这是"`unsafe` 的义务不写在代码里"最具体的一次呈现**：
两个逐字节等价的写法，编译器生成同样的机器码，
一个是健全的、另一个是 UB —— **只有 Miri 能分辨。**

### 第二种陷阱：移动后旧地址被释放

`tests/selfref_ub.rs` 的另一条用例：

```text
error: Undefined Behavior: memory access failed: alloc<N> has been freed,
       so this pointer is dangling
```

★ 附带实测结论：**"移动 = memcpy"不必然当场暴露。**
如果旧地址还活着（比如只是又复制了一份到栈上），Miri **不报错** ——
它只在旧地址**真的失效**时才判定悬垂。
这正是自引用结构最危险的地方：**它可能"碰巧能跑"。**

## 反例三则

### `fail/pin_requires_unpin.rs` → E0277

```text
error[E0277]: `PhantomPinned` cannot be unpinned
  33 |     Pin::new(x)
     |     -------- ^ within `NotUnpin`, the trait `Unpin` is not implemented
     |               for `PhantomPinned`
     = note: consider using the `pin!` macro
             consider using `Box::pin` if you need to access the pinned value outside of the current scope
note: required by a bound in `Pin::<Ptr>::new`
```

★ 编译器指的是**那个字段**（`PhantomPinned`），不是整个类型 ——
与第 12 章 `PhantomData<*const u64>` 让类型 `!Send` 的措辞同类：
**编译器会告诉你"谁该负责"。**

### `fail/move_pinned.rs` → E0596 + E0507

```text
error[E0596]: cannot borrow data in dereference of `Pin<Box<NotUnpin>>` as mutable
  = help: trait `DerefMut` is required to modify through a dereference,
          but it is not implemented for `Pin<Box<NotUnpin>>`

error[E0507]: cannot move out of dereference of `Pin<Box<NotUnpin>>`
```

★ 措辞是 "cannot borrow ... as mutable"，**不是**"这个类型不能移动"。
**`Pin` 用"借用的可变性"来表达"能不能移动"**：
`DerefMut` 那条 impl 对 `!Unpin` **根本不存在** ——
类型系统里少一条 impl，等于运行时多一条保证。

### `fail/future_not_unpin.rs` → E0277（本章最反直觉）

```rust
pub async fn zero_await(a: u64) -> u64 { a }   // 没有借用、没有 await
pub fn try_pin_future() {
    let mut f = zero_await(1);
    let _p: Pin<&mut _> = Pin::new(&mut f);     // ← 照样 E0277
}
```

```text
error[E0277]: `{async fn body of zero_await()}` cannot be unpinned
```

★ **一个不可能自引用的状态机，也被判定为 `!Unpin`。**

原因是一个时序死结：

- "有没有自引用"要等**借用检查之后**才知道；
- `Unpin` 是 **auto trait**，答案必须在**类型层立即给出**。

编译器没法等，于是**一律保守**：所有 `async` 产物都是 `!Unpin`。

**代价**：到处要写 `Box::pin` / `pin!`。
**收益**："自引用 future 一定安全"不需要任何额外规则。

> 对照：`std::future::ready(5)` 是 `Unpin` 的（不是编译器生成的状态机），
> 所以 `Pin::new(&mut ready)` 可以编译。
> **`!Unpin` 不是 `async` 的关键字，而是"编译器生成的状态机"的性质。**

## 交叉验证（可选）

```bash
grep -A 10 '^_make_self_ref:' .evidence/ch19-pin-lib.O3.s
grep 'alias' .evidence/ch19-pin-lib.O3.ll
sed -n '/coroutine layout {/,/storage_conflicts/p' .evidence/ch19-pin-lib.mir
grep -n 'get_unchecked_mut' .evidence/ch19-pin-lib.mir
```

## 待办

- [x] 13 条断言全绿（`verify-all.sh ch19`）
- [x] Miri 两条用例（合法 / UB）落库并接入 `verify-miri.sh`
- [x] 修掉 `src/lib.rs` 里一处**真实的 UB**：`SelfRef::new` 原用
      `&this.data as *const u64`（SharedReadOnly），改为 `addr_of_mut!`
- [ ] 第 20 章（`async` 生命周期与 `Send` 传染）需要新 example
- [ ] 第 21–23 章（AFIT / tokio / mini runtime）均无 example
