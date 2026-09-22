# 第 4 章：自引用结构与 Pin 的前置知识 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch04-pin
scripts/verify-all.sh ch04      # 6 条断言
```

## ★ 证据 1：`Pin<&mut T>` 是**零成本**的

```rust
pub fn plain(x: &mut u64) -> u64 { *x }
pub fn pinned(p: Pin<&mut u64>) -> u64 { *p }
```

LLVM IR：
```llvm
@plain = unnamed_addr alias i64 (ptr), ptr @pinned
```

汇编：
```asm
_pinned:
	ldr	x0, [x0]
	ret

_plain = _pinned
```

**LLVM 判定两者逐位等价，把 `plain` 折叠成了 `pinned` 的 alias。**

**讲法**：`Pin` **不是**运行时包装，它不占空间、不加字段、不产生指令。
它是**类型层的约束**——和生命周期一样，用完就丢。
"`Pin` 有性能开销"是范畴错误。

## ★ 证据 2：移动允许重定位（自引用的根源）

```rust
pub struct Big { pub a: [u64; 4] }
pub fn mov_it(b: Big) -> Big { b }
```

MIR：
```text
fn mov_it(_1: Big) -> Big {
    let mut _0: Big;
    bb0: {
        _0 = move _1;        // ← 移动就是一条 move
        return;
    }
}
```

汇编：
```asm
	ldp	q0, q1, [x0]         ; 从源地址读 32 字节
	stp	q0, q1, [x8]         ; 写到目标地址
	ret
```

这个样本的机器码确实把 32 字节从旧地址复制到了新地址。一般来说，
Rust 的 move 允许重定位，但优化器也可能消除实际复制。
如果 `Big` 里存着一个指向**自己**的指针，这个指针现在就悬垂了。

**这就是自引用结构为什么天然困难**：
Rust 的所有权模型默认"值可以被移动"，而自引用要求"地址永远不变"。
两者直接冲突。

## ★ 证据 3：`Unpin` 是 auto trait，`PhantomPinned` 是零大小的开关

`Pinned` 结构体：
```rust
pub struct Pinned {
    pub data: u64,
    _pin: PhantomPinned,     // ZST —— 一个字节都没多
}
```

但它**不再是 `Unpin`**，于是 `Pin::new` 拒绝编译（`fail/pin_requires_unpin.rs`）：

```text
error[E0277]: `PhantomPinned` cannot be unpinned
   |
23 |     Pin::new(x)
   |     -------- ^ within `Pinned`, the trait `Unpin` is not implemented
   |               for `PhantomPinned`
   |
   = note: consider using the `pin!` macro
           consider using `Box::pin` if you need to access the pinned value
           outside of the current scope
```

★ **这条错误信息就是 `Pin` 的全部意义**：
"这个类型不能保证移动后还有效，所以我不让你从 `&mut T` 造 `Pin<&mut T>`。"

**为什么 `Pin::new` 需要 `Unpin`？**
因为 `Pin::new` 拿的是 `&mut T`——而 `&mut T` 的持有者随时可以
`mem::swap` / `mem::replace` / `ptr::read` 把它移走。
只有当 `T: Unpin`（"移动无害"）时，这个承诺才安全。

## ★ 证据 4：要钉住 `!Unpin` 的值，必须先给它**稳定地址**

```rust
pub fn box_pin(data: u64) -> Pin<Box<Pinned>> { Box::pin(Pinned::new(data)) }
```

汇编里能看到**真正的堆分配**：
```asm
_box_pin:
	bl	__RNvCs...12___rust_alloc      ; ← malloc
	cbz	x0, LBB0_2
	str	x19, [x0]
```

**堆上的地址不会因为栈帧移动而改变** —— 这是 `Box::pin` 能"钉住"的原因。
`pin!` 宏则把值钉在当前栈帧上（配合 `&mut` 借用的生命周期）。

**这是 `Pin` 的核心权衡**：
- `Pin<&mut T>`：零成本，但需要调用者保证地址稳定；
- `Pin<Box<T>>`：一次堆分配，换来"地址天然稳定"。

## ★ 证据 5：`Pin` 真的挡得住移动

```rust
pub fn read_pinned(p: &Pin<Box<Pinned>>) -> u64 { p.data }        // ✅ 只读
pub fn write_pinned(p: &mut Pin<Box<Pinned>>, v: u64) {
    unsafe { p.as_mut().get_unchecked_mut() }.data = v;           // 需要 unsafe
}
```

从 `Pin<Box<T>>` 只能拿到 `&T`（`Deref`）。
要拿 `&mut T`，必须走 `unsafe` 的 `get_unchecked_mut`，
并自己承诺"不会移动它"。

**这正是 `Pin` 的 API 设计意图**：
把"不能移动"这个约定，从**文档要求**变成**类型要求**。
你不用 `unsafe`，就拿不到 `&mut T`；拿不到 `&mut T`，就移不走它。

## 与第 2、3 章的连接

- **第 2 章**：生命周期在 codegen 里不存在。`Pin` 也一样——**纯类型层**。
- **第 3 章**：`&mut T` 在 `T` 上不变。`Pin` 用的是同一个思路：
  通过限制"能拿到什么"，来限制"能做什么"。

## 待办

- [x] 断言 6 条全绿
- [ ] 补一个**真正的自引用结构**（`async` 状态机的雏形），演示"移动后指针悬垂"
- [ ] 第 5 章是实战（树 / 图容器），需要单独的 example
- [ ] 第 18 章（`async` 状态机）会回来用这一章的所有概念
