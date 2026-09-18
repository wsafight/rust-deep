# 4. 自引用结构与 Pin 的前置知识

> 一句话：Rust 的移动是 **memcpy**，而自引用结构里存着"自己的地址"——
> 一移就悬垂。`Pin` 存在的全部理由，就是让"这个值不许移动"变成类型约束。

## 4.0 一个会让你卡住的例子

你大概写过（或者见过）这样的代码：

```rust
struct SelfRef {
    data: String,
    ptr: *const u8,       // 指向 data
}

impl SelfRef {
    fn new(data: String) -> Self {
        let mut s = SelfRef { data, ptr: std::ptr::null() };
        s.ptr = s.data.as_ptr();
        s
    }
}
```

看起来没问题。但把它移一下：

```rust
let a = SelfRef::new(String::from("hello"));
let b = a;                  // ← 移动
println!("{}", unsafe { *b.ptr });   // 读出来的是什么？
```

**`b.ptr` 仍然指向 `a.data` 原来的地址。**
而 `a` 已经被移走了——那个地址现在可能装着别的东西。

这段代码**编译得过**（裸指针不做借用检查），但它是 UB。
Miri 会当场抓到：

```text
error: Undefined Behavior: memory access failed: alloc291 has been freed,
       so this pointer is dangling
```

所以问题不是"裸指针危险"，而是：

> **Rust 的所有权模型允许值被移动，而自引用要求地址永不变。
> 这两件事直接冲突。**

## 4.1 表层解释（官方书会怎么讲）

官方书会说：

- `Pin<P>` 保证被指向的值不会被移动；
- `Unpin` 表示"移动这个类型是安全的"，大部分类型都是 `Unpin`；
- `async` 生成的状态机是 `!Unpin` 的，所以要 `Box::pin`。

这些都对，但它们跳过了最关键的一步：**为什么移动会出问题？**
这一章补上这一步。

## 4.2 编译器眼里的样子

### 4.2.1 移动就是 memcpy —— 在 MIR 里就是一条 `move`

```rust
pub struct Big { pub a: [u64; 4] }
pub fn mov_it(b: Big) -> Big { b }
```

MIR：

```text
fn mov_it(_1: Big) -> Big {
    let mut _0: Big;
    bb0: {
        _0 = move _1;        // ← 就这一条
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

**移动 = 把字节从旧地址搬到新地址。**
不是"改个名字"，是真的搬。

这条事实解释了自引用为什么难：如果 `Big` 里有一个指针指向**自己**，
搬完之后，那个指针还指着旧地址。

> 注意：编译器**可能**优化掉这个 memcpy（比如整个值被内联进寄存器），
> 也可能不优化。**你不能依赖任何一种情况**——
> 这正是为什么自引用必须由类型系统来保证，而不是靠"编译器大概不会移它"。

### 4.2.2 `Pin` 是零成本的

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

`Pin` **不占空间、不加字段、不产生指令**。
它和生命周期一样，是**纯类型层的约束**——用完就丢。

> 所以"`Pin` 有性能开销"是范畴错误。
> 真正有开销的是 `Box::pin`（一次堆分配），
> 而那是为了**稳定地址**，不是 `Pin` 本身。

### 4.2.3 `Unpin` 是 auto trait，`PhantomPinned` 是零大小的开关

```rust
use std::marker::PhantomPinned;

pub struct Pinned {
    pub data: u64,
    _pin: PhantomPinned,     // ZST —— 一个字节都没多
}
```

`Pinned` 的大小和 `u64` 一样。但它**不再是 `Unpin`**。
于是 `Pin::new` 拒绝编译：

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

**为什么 `Pin::new` 需要 `T: Unpin`？**

因为 `Pin::new` 拿到的是 `&mut T`。而持有 `&mut T` 的人随时可以：

```rust
std::mem::swap(&mut *pinned_ref, &mut other);   // 换走
std::mem::replace(&mut *pinned_ref, new_value); // 替换
std::ptr::read(&*pinned_ref);                   // 读走
```

**只要有 `&mut T`，就能移走 `T`。** 所以只有当 `T: Unpin`
（"移动这个类型是无害的"）时，`Pin::new` 才敢给你这个承诺。

### 4.2.4 要钉住 `!Unpin` 的值，必须先给它**稳定地址**

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

两条路：

| 方式 | 地址稳定靠什么 | 代价 |
|---|---|---|
| `Box::pin(x)` | 堆分配，地址天然稳定 | 一次分配 |
| `pin!(x)` | 钉在当前栈帧，配合 `&mut` 的生命周期 | 零分配，但借用期受限 |

**这是 `Pin` 的核心权衡**：零成本的 `Pin<&mut T>` 需要调用者保证地址稳定；
`Pin<Box<T>>` 用一次分配换来"地址天然稳定"。

### 4.2.5 `Pin` 真的挡得住移动

```rust
pub fn read_pinned(p: &Pin<Box<Pinned>>) -> u64 { p.data }        // ✅ 只读
pub fn write_pinned(p: &mut Pin<Box<Pinned>>, v: u64) {
    // SAFETY: 只改 data，不移动整个 Pinned
    unsafe { p.as_mut().get_unchecked_mut() }.data = v;
}
```

从 `Pin<Box<T>>` 只能拿到 `&T`（通过 `Deref`）。
要拿 `&mut T`，必须走 `unsafe` 的 `get_unchecked_mut`，
并**自己承诺不会移动它**。

**这就是 `Pin` 的 API 设计意图**：
把"不能移动"从**文档要求**变成**类型要求**。
不用 `unsafe` 就拿不到 `&mut T`；拿不到 `&mut T`，就移不走它。

## 4.3 为什么必须这样设计

### 为什么不能"移动时自动修正自引用指针"？

因为 Rust 没有运行时类型信息，也没有 GC。
要知道"哪些字段是指向自己的指针"，需要跟踪整个指针图——
那是 GC 语言（或者有 `memmove` 回调的 C++ 类型）才做的事。
Rust 选择了另一个方向：**不修，而是不许移**。

### 为什么 `Unpin` 是 auto trait，而 `Pin` 是显式包装？

`Unpin` 是 auto trait —— 大部分类型（`u64`、`String`、`Vec<T>`……）
**自动**是 `Unpin`，因为它们的移动确实无害（不存自引用）。
只有你**显式**加了 `PhantomPinned` 才会变成 `!Unpin`。

这个设计的好处：**"不能移动"是例外，不是常态**。
普通代码完全不用管 `Pin`；只有写自引用类型（`async` 状态机、
自引用 future、某些 intrusive 容器）的人才会遇到它。

### 为什么 `PhantomPinned` 是零大小的？

因为它**只需要传递一个类型层的事实**（"这个类型是 `!Unpin`"），
不需要存任何数据。它和 `PhantomData<T>` 是同一类东西：
**零大小的类型层开关**。

对比第 3 章的 `PhantomData<Cell<&'a ()>>` ——
那个也是零大小，但它改变的是 **variance**。
两个 `Phantom*` 都是"不占空间但参与类型推导"的工具。

## 4.4 反直觉的点

### 反直觉之一：`Pin` 不保证"值不会被移动"，它只保证"你不能移动它"

`Pin` 是一个**约定**，不是魔法。你可以这样绕过它：

```rust
let pinned: Pin<Box<Pinned>> = Box::pin(Pinned::new(1));
// 把 Box 里的东西换掉 —— 这是允许的！Box 的地址没变
```

`Pin<Box<T>>` 保证的是"`Box` 指向的那块内存不会被释放或替换"，
不保证"里面的字节不会被改"。**改内容是可以的**（`Pin<Box<T>>` 给 `DerefMut`
在某些条件下…… 不，给的是 `Deref`；但 `Pin<&mut T>` 确实可以在
`T: Unpin` 时给 `DerefMut`）。

准确的说法：**`Pin<P>` 保证的是 `P` 指向的值不会被移动到别处**，
而 `P` 本身（那个 `Box` 或那个引用）可以被移动——移动 `Box` 只是搬走一个指针，
堆上的值没动。

### 反直觉之二：`Pin` 在汇编里**什么都不剩**

见 4.2.2 —— `plain` 和 `pinned` 被 LLVM 折叠成 alias。

**所以 `Pin` 是"零成本抽象"的又一个例子**：
你付出的成本是**编译期的约束**（不能随便拿 `&mut`），
换来的是**运行时的零开销** + **健全性**。

这和第 1、2 章的结论是同一条：
**Rust 的安全保证是编译期的，不落到运行时。**

### 反直觉之三：`!Unpin` 不等于"危险"

```rust
struct Pinned { data: u64, _pin: PhantomPinned }
```

这个类型完全安全——它只是**不能被 `Pin::new` 包装**。
如果你永远不移动它（比如它就活在栈上某个固定位置，或者放在 `Box` 里），
它工作得好好的。

`!Unpin` 的意思是"**移动它需要额外的证明**"，不是"这个类型有问题"。
和第 12 章的 `!Send` / `!Sync` 是同一种语气：
**"这个类型只在一个更窄的条件下安全"。**

## 4.5 亲手验证

```bash
tools/evidence.sh ch04-pin
scripts/verify-all.sh ch04

# 反例
rustc --edition 2024 --crate-type=lib examples/ch04-pin/fail/pin_requires_unpin.rs
```

**怎么算验证成功**：

1. LLVM IR 里出现 `@plain = ... alias ... ptr @pinned`
   —— `Pin` 是零成本的；
2. MIR 里 `mov_it` 的函数体只有 `_0 = move _1` —— 移动就是一条指令；
3. `_box_pin` 的汇编里有 `bl ...___rust_alloc` —— 堆分配是真的；
4. 反例报 E0277，且信息里有 `PhantomPinned cannot be unpinned`。

```bash
scripts/verify-all.sh ch04      # 6 条断言
```

## 4.6 与 unsafe 的关系

`Pin` 是**用类型系统表达"这个值不会被移动"**的尝试。
一旦你走 `unsafe`（`get_unchecked_mut`、`Pin::new_unchecked`），
这个约定就落回**你**头上。

```rust
// SAFETY: 我承诺不会移动这个值
let mut_ref = unsafe { pinned.get_unchecked_mut() };
```

**这个 SAFETY 注释不是形式主义。** 如果你违反了它，
后果不是"程序崩溃"，而是**任何事都可能发生**——
因为 `async` 状态机（第 18 章）会把"自己的地址"存在字段里，
移动它会让那些地址失效。

Miri 能抓到这类错误：

```text
error: Undefined Behavior: memory access failed: alloc291 has been freed,
       so this pointer is dangling
```

**这一章是第 18–20 章（异步）的地基。**
`async fn` 生成的 future 就是一个自引用状态机，
`Pin` 就是为了让它能安全存在而设计的。

## 4.7 小结

- **移动 = memcpy**。MIR 里就是一条 `move`，汇编里就是 `ldp`/`stp`。
  不是"改名字"，是搬字节。
- **自引用结构与移动天然冲突**：存了"自己的地址"，一移就悬垂。
- **`Pin` 是零成本的类型层约束**：`Pin<&mut T>` 与 `&mut T`
  生成同一个函数（LLVM 折叠成 alias）。
- **`Unpin` 是 auto trait**：大部分类型自动是 `Unpin`，
  加 `PhantomPinned`（零大小）就变成 `!Unpin`。
- **`Pin::new` 需要 `T: Unpin`**，因为 `&mut T` 的持有者随时能移走它。
  要钉住 `!Unpin` 的值，必须先给稳定地址：`Box::pin` 或 `pin!`。
- **`Pin` 不保证"值不会被移动"，只保证"你不能移动它"**。
  它把约定从文档搬到了类型系统里。
- **`!Unpin` 不等于危险**，只是"移动需要额外证明"——
  和第 12 章的 `!Send` / `!Sync` 是同一种语气。

第 1 部分到这里结束。接下来进入第 2 部分：trait 系统。
我们从"关联类型 vs 泛型参数"开始——这个选择比大多数人想的更本质。
