# 4. 自引用结构与 Pin 的前置知识

> 一句话：Rust 允许把值重新放到另一个地址，而自引用结构里存着
> "自己的地址"——一旦重定位，内部指针就可能失效。`Pin` 把
> “从此不能再安全移动”变成了类型约束。

## 先把语法认清

`Pin<P>` 包装一个指针类型 `P`，限制通过该指针移动其目标值。普通类型实现
`Unpin`，Pin 对它们几乎没有约束；包含 `PhantomPinned` 的类型可选择 `!Unpin`，
要求初始化后保持地址稳定。`Box::pin(value)` 和 `pin!(value)` 分别提供堆上与
当前作用域内的稳定位置。

一个好记的画面是：`Pin<Box<T>>` 固定的是**货物所在的货位**，不是手里的
提货单。提货单可以从 `p` 移给 `q`，堆上的货物不能被安全代码搬走。

### 放到业务里：异步状态机与侵入式结构

异步函数挂起时，状态机里的一个字段可能引用另一个字段；侵入式链表、
注册到操作系统的 IO 请求也可能把对象地址保存到别处。如果对象随后被移动，
这些地址就会失效。业务代码通常不手写自引用，而是通过 `Pin<Box<F>>`、
框架提供的 pinned API 或投影库维护这个不变量。

```rust,ignore
let future = Box::pin(read_response(socket));
runtime.spawn(future); // 移动的是 Box 句柄，不是堆上的状态机
```

这里 `Box` 提供稳定地址，`Pin` 限制安全代码移动里面的 future；两者职责不同。

## 4.0 一个会让你卡住的例子

你大概写过（或者见过）这样的代码：

```rust
struct SelfRef {
    data: [u8; 5],
    ptr: *const u8,       // 指向 data
}

impl SelfRef {
    fn new(data: [u8; 5]) -> Self {
        let mut s = SelfRef { data, ptr: std::ptr::null() };
        s.ptr = s.data.as_ptr();
        s
    }
}
```

看起来没问题。但把它移一下：

```rust,ignore
let a = SelfRef::new(*b"hello");
let b = a;                  // ← 移动
println!("{}", unsafe { *b.ptr });   // 读出来的是什么？
```

`data` 现在是结构体内联字段，不是另一次堆分配。`ptr` 记录的是构造时
`s.data` 的地址；返回 `s`、再把 `a` 移给 `b` 都允许结构体换位置，裸指针
不会自动跟着修正。不能依赖编译器碰巧做了返回值优化。

一旦解引用的仍是旧地址，这段代码就是 UB。裸指针绕过了静态借用检查；
Miri 可以在实际发生重定位并使旧位置失效的测试路径上抓到它：

```text
error: Undefined Behavior: memory access failed: alloc291 has been freed,
       so this pointer is dangling
```

所以问题不是"裸指针危险"，而是：

> **Rust 的所有权模型允许值被移动，而自引用要求地址永不变。
> 这两件事直接冲突。**

## 4.1 先把常见说法摆上桌

通常会这样概括：

- `Pin<P>` 保证被指向的值不会被移动；
- `Unpin` 表示"移动这个类型是安全的"，大部分类型都是 `Unpin`；
- `async` 生成的状态机是 `!Unpin` 的，所以要 `Box::pin`。

这些都对，但它们跳过了最关键的一步：**为什么移动会出问题？**
这一章补上这一步。

## 4.2 编译器眼里的样子

### 4.2.1 移动允许重定位 —— MIR 里是一条 `move`

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

这个样本的机器码确实把 32 字节从源地址复制到目标地址。更一般地说，
Rust 的 move **允许值换地址**，但编译器也可能消除实际拷贝；语言不承诺
移动后地址保持不变。

这条事实解释了自引用为什么难：如果 `Big` 里有一个指针指向**自己**，
搬完之后，那个指针还指着旧地址。

> 注意：编译器**可能**优化掉这个 memcpy（比如整个值被内联进寄存器），
> 也可能不优化。**你不能依赖任何一种情况**——
> 这正是为什么自引用必须由类型系统来保证，而不是靠"编译器大概不会移它"。

### 4.2.2 `Pin` 是零成本的

```rust,ignore
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

```rust,ignore
std::mem::swap(&mut *pinned_ref, &mut other);   // 换走
std::mem::replace(&mut *pinned_ref, new_value); // 替换
std::ptr::read(&*pinned_ref);                   // 读走
```

**只要有 `&mut T`，就能移走 `T`。** 所以只有当 `T: Unpin`
（"移动这个类型是无害的"）时，`Pin::new` 才敢给你这个承诺。

### 4.2.4 要钉住 `!Unpin` 的值，必须先给它**稳定地址**

```rust,ignore
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

```rust,ignore
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

### 反直觉之一：移动 Pin 句柄，不等于移动被钉住的值

`Pin` 约束的是安全代码通过指针能做什么，不是给内存施法。下面这种移动
完全合法：

```rust,ignore
let pinned: Pin<Box<Pinned>> = Box::pin(Pinned::new(1));
let moved = pinned; // 移动 Pin<Box<_>> 句柄，堆上的 Pinned 没动
```

移动的只是拥有指针的句柄。只要被钉住的值还存活，安全代码就不能把
`!Unpin` 的 `T` 从当前位置移走或替换掉。不过，Pin 不等于只读：对不涉及
结构固定的不变量，仍可通过安全投影 API 修改；使用 `get_unchecked_mut` 时，
则由调用者保证不会移动受结构固定保护的部分。

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

```rust,ignore
struct Pinned { data: u64, _pin: PhantomPinned }
```

这个类型本身完全安全，而且在**尚未被 pin、尚未建立地址相关状态之前**仍可
正常移动。`!Unpin` 真正表达的是：一旦通过 Pin 承诺了位置稳定，就不能再
借助安全 API 随意解除这层保护。`Box<T>` 本身也不等于 pin，只有
`Pin<Box<T>>` 才建立这份承诺。

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
2. MIR 里 `mov_it` 的函数体只有 `_0 = move _1`；当前汇编样本把值复制到
   返回位置，说明 Rust 不承诺 move 后地址不变；
3. `_box_pin` 的汇编里有 `bl ...___rust_alloc` —— 堆分配是真的；
4. 反例报 E0277，且信息里有 `PhantomPinned cannot be unpinned`。

```bash
scripts/verify-all.sh ch04      # 6 条断言
```

## 4.6 与 unsafe 的关系

`Pin` 是**用类型系统表达"这个值不会被移动"**的尝试。
一旦你走 `unsafe`（`get_unchecked_mut`、`Pin::new_unchecked`），
这个约定就落回**你**头上。

```rust,ignore
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
`async fn` 生成的 future **可能**形成自引用状态机，
`Pin` 让这类地址敏感状态可以通过安全接口被轮询。

## 4.7 小结

- **移动允许值被重定位**。本章样本的 MIR 是一条 `move`，汇编表现为
  `ldp`/`stp`；优化器也可能消除实际拷贝，所以不能依赖地址保持不变。
- **自引用结构与移动天然冲突**：存了"自己的地址"，一移就悬垂。
- **`Pin` 是零成本的类型层约束**：`Pin<&mut T>` 与 `&mut T`
  生成同一个函数（LLVM 折叠成 alias）。
- **`Unpin` 是 auto trait**：大部分类型自动是 `Unpin`，
  加 `PhantomPinned`（零大小）就变成 `!Unpin`。
- **`Pin::new` 需要 `T: Unpin`**，因为 `&mut T` 的持有者随时能移走它。
  要钉住 `!Unpin` 的值，必须先给稳定地址：`Box::pin` 或 `pin!`。
- **移动 `Pin<Box<T>>` 句柄不等于移动 `T`**。对 `!Unpin` 的 `T`，
  安全代码不能把被钉住的值从当前位置移走；越过该边界需要 unsafe 证明。
- **`!Unpin` 不等于危险**，只是"移动需要额外证明"——
  和第 12 章的 `!Send` / `!Sync` 是同一种语气。

第 1 部分到这里结束。接下来进入第 2 部分：trait 系统。
我们从"关联类型 vs 泛型参数"开始——这个选择比大多数人想的更本质。
