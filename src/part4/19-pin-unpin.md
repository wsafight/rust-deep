# 19. `Pin` 与 `Unpin` 为什么存在

> 一句话：`Pin` 不是"防止值被移动"的魔法，而是**一个权限收缩装置**——
> 它把"移动一个值"所需要的两把钥匙（**所有权**和 **`&mut`**）收走，
> 于是"不能移动"从一条**文档约定**变成了**类型约束**。
> 它服务于一类共同需求：值一旦进入地址敏感状态，就不能再被安全移动；
> 自引用的 `async` 状态机是最重要的应用之一。

## 先把语法认清

`Pin<&mut T>` 或 `Pin<Box<T>>` 表示从安全代码不能再移动被指向的 `T`。
若 `T: Unpin`，这个限制可以安全解除；`!Unpin` 类型则只能通过保证不移动
字段的投影 API 修改。Pin 保证的是位置稳定性，不是只读，也不是对象地址永远
不会被底层 unsafe 代码破坏。

仍然用搬运来记：`Pin<Box<T>>` 固定的是**货物**，不是装着地址的**提货单**。
提货单可以移动，货物一旦进入地址敏感状态就必须留在原位。

### 放到业务里：等待中的 IO Future

网络驱动把 future 注册到事件循环后，future 内部可能保存指向自身缓冲区或
状态字段的指针。executor 可以移动 `Pin<Box<Future>>` 这个句柄，却不能移动
堆上的 future 本体。业务层通常只会看到 `Box::pin` 或宏生成的投影代码，
但理解 Pin 能解释为什么某些 future 不能直接借成 `&mut T`。

```rust
let request = Box::pin(read_response(socket));
queue.push(request); // 队列移动的是 Pin<Box<_>>，不是 future 本体
```

这也是为什么“Pin 以后还能不能 move”必须先问清移动的是哪一层：句柄可以走，
被钉住的值不能被安全代码从原位置搬走。

第 4 章讲过 `Pin` 是什么（零成本、`Unpin` 是 auto trait、`PhantomPinned` 是开关）。
这一章回答**为什么它必须存在**：我们会真的构造一个自引用结构，
看着它编译通过、跑出正确结果、然后被 Miri 判定为 UB ——
并找出**同一个逻辑、两种写法**里，哪一种是 sound 的。

## 19.0 一个会让你卡住的例子

`async fn` 的状态机里，一个跨 `await` 的局部变量**可以借用另一个**：

```rust
pub async fn borrow_across_await() -> u64 {
    let x = String::from("hi");
    let r = &x;                     // ← r 借用 x
    std::future::ready(()).await;   // ← 这个 await 之后，r 还活着
    r.len() as u64
}
```

这看起来平平无奇。但它意味着：**这个 future 的状态机里，
一个字段（`&String`）指向另一个字段（`String`）** ——
也就是它**指向自己**。

而 Rust 的移动允许重定位（第 4 章）：状态机被搬到别处时，
那个内部指针还指着**旧地址**。

于是你遇到本章的两类"卡住"：

**（A）你写了个自引用结构，它编译过了、也"跑对了"：**

```rust
pub struct SelfRef {
    pub data: u64,
    self_ref: *mut u64,        // 指向自己的 data
    _pin: PhantomPinned,
}
```

裸指针不做借用检查，所以这段代码**编译得过**。
把它构造出来、读一下，也**打印出正确的数字**。
但只要它被移动过，那就是 UB —— 而 `cargo run` 不一定会告诉你。

**（B）你想把 future 钉住，编译器拒绝：**

```rust
let mut f = borrow_across_await();
let p: Pin<&mut _> = Pin::new(&mut f);   // ← 编译不过
```

```text
error[E0277]: `{async fn body of borrow_across_await()}` cannot be unpinned
   = note: consider using the `pin!` macro
           consider using `Box::pin` if you need to access the pinned value
           outside of the current scope
```

**为什么拒绝？** "它不能 unpin"这句话到底在说什么？
这一章要把它拆开——并且要回答一个更尖锐的问题：

> **如果自引用是 UB，那 `async` 是怎么活下来的？**
> 标准库难道不该有一个 `unsafe` 的漏洞吗？

## 19.1 先把常见说法摆上桌

通常会这样概括：

- `Pin<P>` 保证被指向的值不会被移动；
- `Unpin` 表示"移动这个类型是安全的"，大部分类型都是 `Unpin`；
- `async` 生成的状态机是 `!Unpin`，所以要 `Box::pin`；
- 自引用指针的创建方式必须与后续访问相容；若后续还要通过独占路径写入，
  不能继续使用此前从共享引用派生、且已被该写入作废的指针。

前三条第 4 章讲过。**最后一条**才是本章的正题 ——
它是 `Pin` 的 soundness 边界，而它**在代码里完全看不出来**。

## 19.2 编译器眼里的样子

### 19.2.1 自引用在运行时是什么：**一条 `stp`**

先看一个真的能跑的自引用结构。`examples/ch19-pin/src/lib.rs`：

```rust
pub struct SelfRef {
    pub data: u64,
    self_ref: *mut u64,     // 指向 self.data
    _pin: PhantomPinned,    // 让它 !Unpin
}

impl SelfRef {
    pub fn new(data: u64) -> Pin<Box<SelfRef>> {
        let mut b = Box::pin(SelfRef { data, self_ref: std::ptr::null_mut(), _pin: PhantomPinned });
        let this: &mut SelfRef = unsafe { b.as_mut().get_unchecked_mut() };
        this.self_ref = std::ptr::addr_of_mut!(this.data);
        b
    }
}
```

O3 的汇编（`aarch64-apple-darwin` / `rustc 1.98.1` / `-O` / `#[unsafe(no_mangle)]`）：

```asm
_make_self_ref:
	mov	x19, x0              ; 先保住参数 data
	mov	w0, #16              ; 16 字节 = u64(8) + 指针(8)
	mov	w1, #8               ; align 8
	bl	___rust_alloc_zeroed ; ← 堆分配：地址稳定的来源
	cbz	x0, LBB1_2           ; 分配失败 → 走 panic
	stp	x19, x0, [x0]        ; ★ data 和"指向自己的地址"一起写进去
	ret
```

**`stp x19, x0, [x0]` 这一条就是自引用的全部运行时形态。**

拆开看：`x0` 是刚分配到的地址；`x19` 是 `data`。
这条指令同时做两件事：

| 写入 | 含义 |
|---|---|
| `[x0] = x19` | `data` 字段 = 传入的 data |
| `[x0 + 8] = x0` | `self_ref` 字段 = **分配到的地址本身** |

**没有魔法，没有运行时登记，就是"把自己的地址存进自己"。**

对照 O0（未优化）版本，能看清这两件事本来是分开的：

```asm
	ldr	x9, [sp, #8]     ; this: &mut SelfRef
	ldr	x8, [sp, #24]    ; data
	str	x8, [x9]         ; this.data = data
	mov	x8, x9
	str	x8, [x9, #8]     ; this.self_ref = &this.data
```

O3 把两条 `str` 合并成了一条 `stp` —— 这就是"自引用"和"两个普通字段"
在机器码层面**没有区别**的原因：它只是两个数字，其中一个恰好是个地址。

> ⚠️ 符号名里的 hash（如 `CsrtIYgyWToU`）**每次编译都会变**，
> 引用汇编时不要把它当常量。

读回来也一样朴素：

```asm
_read_self_ref:
	ldr	x8, [x0]        ; 从 &Pin<Box<SelfRef>> 取出 Box 指针
	ldr	x8, [x8, #8]    ; 取 self_ref 字段（偏移 8）
	ldr	x0, [x8]        ; 顺着它读 data
	ret
```

**两次解引用。** 第二次解引用的目标，就是第一次解引用拿到的那块内存本身。

### 19.2.2 自引用状态机：MIR 里的 `coroutine layout`

现在把 19.0 那个 `async fn` 的 MIR 打出来（`tools/evidence.sh ch19-pin`）：

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

★ **`_s1: &String` 就是 19.2.1 里那个 `self_ref`。**

第 18 章展示了挂起点如何进入状态机布局；这里看到的是**同一件事的另一面**：
`Suspend0` 变体里同时装着 `_s0`（`String`）和 `_s1`（`&String`）——
一个指向另一个。**这就是编译器必须保守的原因**（19.4 展开）。

### 19.2.3 `Pin` 挡住移动的两条路

一个值能被移动，只有两条路：**拿到所有权**，或者**拿到 `&mut`**。
`Pin` 把两条路同时堵死。`fail/move_pinned.rs`：

```rust
pub fn try_get_mut(p: &mut Pin<Box<NotUnpin>>) {
    let inner: &mut NotUnpin = &mut *p;     // ① E0596
}

pub fn try_move_out(p: Pin<Box<NotUnpin>>) {
    let inner: NotUnpin = *p;               // ② E0507
}
```

```text
error[E0596]: cannot borrow data in dereference of `Pin<Box<NotUnpin>>` as mutable
  = help: trait `DerefMut` is required to modify through a dereference,
          but it is not implemented for `Pin<Box<NotUnpin>>`

error[E0507]: cannot move out of dereference of `Pin<Box<NotUnpin>>`
```

★ **注意错误的措辞**：编译器说的是 "cannot borrow ... as mutable"，
**不是**"这个类型不能移动"。

这是 `Pin` 设计里最精妙的一点：

> **它用"借用的可变性"来表达"能不能移动"。**

`Pin<P>: DerefMut` 只在 `P::Target: Unpin` 时可用。对 `!Unpin` 的类型，
`DerefMut` 这一条 impl **根本不存在** —— 于是 `&mut *p` 无从构造，
移动也就无从谈起。**类型系统里少一条 impl，等于运行时多一条保证。**

### 19.2.4 `Unpin` 是 auto trait，`PhantomPinned` 是零大小的开关

```rust
pub struct NotUnpin { pub a: u64, _p: PhantomPinned }
```

`PhantomPinned` 是 ZST，`NotUnpin` 的大小和 `Plain { a: u64 }` **完全一样**。
但 `Pin::new` 拒绝编译（`fail/pin_requires_unpin.rs`）：

```text
error[E0277]: `PhantomPinned` cannot be unpinned
  33 |     Pin::new(x)
     |     -------- ^ within `NotUnpin`, the trait `Unpin` is not implemented
     |               for `PhantomPinned`
     = note: consider using the `pin!` macro
             consider using `Box::pin` if you need to access the pinned value outside of the current scope
note: required by a bound in `Pin::<Ptr>::new`
```

★ **注意编译器指的是哪个类型**：它说的不是 "`NotUnpin` is not Unpin"，
而是 "**`PhantomPinned`** cannot be unpinned" —— 它指出了**该负责的那个字段**。
这与第 12 章 `PhantomData<*const u64>` 让类型 `!Send` 的措辞是同一类：
**编译器会告诉你"谁该负责"。**

### 19.2.5 `Pin` 本身仍然零成本

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
`Pin` 不占空间、不加字段、不产生指令。

> 所以"`Pin` 有性能开销"是范畴错误。有开销的是 `Box::pin`（一次堆分配），
> 而那是为了**地址稳定**，不是 `Pin` 本身。

## 19.3 为什么必须这样设计

### 为什么不能"移动时自动修正自引用指针"

因为要知道"哪些字段指向自己"，需要跟踪整个指针图 ——
那是 GC 语言（或带 `memmove` 回调的 C++ 类型）才做的事。
Rust 没有运行时类型信息，也没有 GC。它选了另一个方向：

> **不修正，而是不许移。**

而"不许移"这件事**不能靠约定**——约定会被 `unsafe`、被新代码、
被一个"看起来无关"的改动打破。所以它必须落到类型上。

### 为什么 `Pin` 用"借用的可变性"来表达"不能移动"

因为这是**唯一一个不需要新语言特性**的表达方式。

"一个值不会被移动"本质上是个**位置性质**（它关于值在内存中的地址），
而 Rust 的类型系统描述的是**类型**，不是地址。`Pin` 的办法是换个角度问：

> 不是"这个值会不会被移动"，而是"**谁能拿到移动它的手段**"。

移动需要所有权或 `&mut`。于是：

| 你想做的事 | `Pin` 给出的 |
|---|---|
| 拿 `&T`（读） | ✅ `Deref` |
| 拿 `&mut T`（改 / 移） | ❌ 除非 `T: Unpin`，或者你 `unsafe` |
| 拿所有权（移出） | ❌ 永远不行 |

**收缩权限，而不是新增规则。** 这就是为什么 `Pin` 的 API 那么小，
也解释了 19.2.3 里那条"看起来答非所问"的错误信息。

### 为什么 `Unpin` 是 auto trait

`Unpin` 的含义是"**移动这个类型是无害的**"。
绝大部分类型（`u64`、`String`、`Vec<T>`……）确实如此——
它们不存自引用，移动就是搬字节。

所以正确的默认值是 **`Unpin`**：把"不能移动"做成**例外**。
你只有显式加 `PhantomPinned` 才会掉出这个默认。

这个设计的收益极大：**写普通 Rust 的人一辈子不用碰 `Pin`**。
代价是 19.4 要讲的那个保守性。

## 19.4 反直觉的点

### 反直觉之一：**所有** `async fn` 的 future 都是 `!Unpin`，连没有 `await` 的也是

直觉上你会以为：`Unpin` 是 auto trait，"有没有自引用"是能看出来的，
所以**只有自引用的状态机才该是 `!Unpin`**。

**实测不是这样。**（`fail/future_not_unpin.rs`）

```rust
pub async fn zero_await(a: u64) -> u64 { a }   // 没有借用、没有 await

pub fn try_pin_future() {
    let mut f = zero_await(1);
    let _p: Pin<&mut _> = Pin::new(&mut f);     // ← 照样 E0277
}
```

```text
error[E0277]: `{async fn body of zero_await()}` cannot be unpinned
   = note: consider using the `pin!` macro
           consider using `Box::pin` if you need to access the pinned value
           outside of the current scope
```

**一个不可能自引用的状态机，也被判定为 `!Unpin`。**

这是当前编译器对匿名 async 状态机采取的保守语义：
1.98.1 上，编译器生成的 `async` 产物不会自动实现 `Unpin`，
即使具体函数没有形成自引用。不要把这个观察解释成借用检查与 auto trait
推导之间必然存在某种"时序死结"；对读者可靠的结论是 API 行为本身。

**代价**就是你到处要写 `Box::pin` / `pin!`；
**收益**是"自引用 future 一定安全"这件事**不需要任何额外规则**——
没有"这个 future 是不是自引用的"这种需要判断的东西。

> 对照：`std::future::ready(5)` 是 `Unpin` 的（它不是编译器生成的状态机），
> 所以 `Pin::new(&mut ready)` 可以编译。
> **`!Unpin` 不是 `async` 的关键字，而是"编译器生成的状态机"的性质。**

### 反直觉之二：同样的机器码可能对应不同的别名有效性

这是本章最值得记住的一条。

自引用指针有两种写法：

```rust
this.self_ref = std::ptr::addr_of!(this.data);      // A
this.self_ref = std::ptr::addr_of_mut!(this.data);  // B
```

**两者的汇编完全一样**（都是取 `this + 0` 的地址，没有别的指令）。
在本章构造并随后改写字段的访问序列中，Miri 给出相反判定：

| 写法 | Miri（Stacked Borrows） |
|---|---|
| A `addr_of!` | ❌ `Undefined Behavior: attempting a read access ... but that tag does not exist in the borrow stack for this location` |
| B `addr_of_mut!` | ✅ 通过 |

**为什么？** 因为指针的**出处**决定了它的**权限**：

- `addr_of!(place)` 派生的是 **SharedReadOnly** 权限 ——
  之后任何一次对 `data` 的写入都会把它**弹掉**（共享借用不允许写入者）；
- `addr_of_mut!(place)` 派生的是 **Unique** 权限 —— 写入不会作废它。

本章实现会在建立自引用前后使用独占路径初始化字段，因此必须保证保存下来的
指针没有被这些写入作废；B 与这段具体访问序列相容。`addr_of!` 本身并不
普遍制造 UB，关键是指针来源与后续访问的组合。

★ **这就是 `unsafe` 的本质**（第 24 章）：

> 代码里**没有**任何一处写着"这个指针的权限是什么"。
> 两段最终机器码可以逐字节等价，但其中一段源码的访问历史违反别名模型。
> **这份契约只存在于你脑子里——所以它必须写进 `SAFETY` 注释，并由 Miri 检查。**

同一类陷阱还有第二种（`fail` 之外的运行期演示，见 `tests/selfref_ub.rs`）：
**把值搬到另一个地址，然后释放旧地址**。Miri 报
`memory access failed: alloc<N> has been freed, so this pointer is dangling`。

> 附带一条实测结论：**移动后的地址问题不必然当场暴露。**
> 如果旧地址还活着（比如只是栈上又复制了一份），Miri 可能不报错 ——
> 它只在旧地址**真的失效**时才判定悬垂。
> 这正是自引用结构最危险的地方：**它可能"碰巧能跑"，然后在某次重构后爆炸。**

### 反直觉之三：`Pin` 不保证"值不会被移动"，只保证"你移动不了它"

`Pin<Box<T>>` 保证的是"`Box` 指向的那块内存不会被释放或替换"，
**不保证**"里面的字节不会被改"。

```rust
let p: Pin<Box<NotUnpin>> = Box::pin(...);
let q = p;   // ← 移动 Pin<Box<_>> 本身：完全合法
```

移动的只是那个 `Box`（8 字节的指针），堆上的值一动没动。
`Pin` 是关于**被指物**的承诺，不是关于指针的。

（这也是 `Box::pin` 能成立的全部依据：**堆地址不随栈帧移动**。
`tests/selfref.rs` 里有一条用例断言"移动 `Pin<Box<_>>` 前后地址不变"。）

### 反直觉之四：`!Unpin` 不等于"危险"

```rust
pub struct NotUnpin { pub a: u64, _p: PhantomPinned }
```

这个类型**完全安全**。它只是不能被 `Pin::new` 包装 ——
只要你永远不移动它（让它待在 `Box` 里，或者钉在栈上的固定位置），
它工作得好好的。

`!Unpin` 的意思是"**移动它需要额外的证明**"，不是"这个类型有问题"。
和第 12 章的 `!Send` / `!Sync` 是同一种语气：
**"这个类型只在一个更窄的条件下安全"。**

## 19.5 亲手验证

```bash
tools/evidence.sh ch19-pin
scripts/verify-all.sh ch19      # 13 条断言

# ★ 反例：三条"必须编译不过"
rustc --edition 2024 --crate-type=lib examples/ch19-pin/fail/pin_requires_unpin.rs
rustc --edition 2024 --crate-type=lib examples/ch19-pin/fail/move_pinned.rs
rustc --edition 2024 --crate-type=lib examples/ch19-pin/fail/future_not_unpin.rs

# ★ 自引用的运行时形态（一条 stp）
grep -A 10 '^_make_self_ref:' .evidence/ch19-pin-lib.O3.s

# ★ 零成本：两个符号折叠成一个
grep 'alias' .evidence/ch19-pin-lib.O3.ll

# ★ 自引用状态机的布局
sed -n '/coroutine layout {/,/storage_conflicts/p' .evidence/ch19-pin-lib.mir
```

**怎么算验证成功**：

1. `_make_self_ref` 的汇编里有 `stp x19, x0, [x0]`
   —— **自引用 = 把自己的地址写进自己**，就这一条；
2. LLVM IR 里有 `@plain = ... alias ... ptr @pinned`
   —— `Pin` 零成本；
3. MIR 里有 `field _s1: &String` 和 `Suspend0 (3): [_s0, _s1, _s2]`
   —— 状态机里一个字段指向另一个字段；
4. 三个反例分别报 **E0277**（`PhantomPinned cannot be unpinned`）、
   **E0596 + E0507**（两条移动的路都被堵）、
   **`cannot be unpinned`**（连没有 `await` 的 `async fn` 也是 `!Unpin`）。

★ **Miri 部分单独跑**（需要 nightly，见附录 A）：

```bash
scripts/verify-miri.sh      # 5 条：ch25 两条 + ch19 两条 + 前置检查
```

它验证的是 19.4 里那条最关键的结论：

- `tests/selfref.rs` —— `addr_of_mut!` 派生的自引用指针，**Miri 必须通过**；
- `tests/selfref_ub.rs` —— `addr_of!` 派生（写后失效）+ 移动后悬垂，
  **Miri 必须报 `Undefined Behavior`**。

**这两条是本章真正的证据。** 普通 `cargo test` 对它们完全无感 ——
这正是"UB 不保证当场崩"的含义。

## 19.6 与 unsafe 的关系

`Pin` 是"用类型系统表达'这个值不会被移动'"的尝试。
一旦走 `unsafe`（`get_unchecked_mut`、`Pin::new_unchecked`），
这个约定就**落回你头上**。

本章的 `make_self_ref` 里那两行 `unsafe` 是全书最典型的样本之一：

```rust
let this: &mut SelfRef = unsafe { b.as_mut().get_unchecked_mut() };
this.data = data;
this.self_ref = std::ptr::addr_of_mut!(this.data);
```

**这段代码的 `unsafe` 义务有两条，缺一不可：**

1. **不移动 `*this`** —— 这是 `Pin` 的约定（`get_unchecked_mut` 的前提）；
2. **让自引用指针从 `&mut` 派生**（`addr_of_mut!`，而不是 `addr_of!`）
   —— 这是 Stacked Borrows 的约定（19.4 反直觉之二）。

第 2 条不会由借用检查器完整证明，也不会在汇编里留下直接痕迹。
Miri 可以在执行具体测试路径时检查这类别名错误。

```bash
cargo +nightly miri test -p ch19-pin --test selfref      # 必须通过
cargo +nightly miri test -p ch19-pin --test selfref_ub   # 必须失败
```

> **这是本书里"`unsafe` 的代价"最具体的一次呈现**：
> 你写下的每一个 `unsafe`，都同时购买了一张**别人看不见的**义务清单。
> Miri 是检查其中一部分问题的重要工具 —— 而它只覆盖 Miri 能理解的那些规则
> （Stacked Borrows 目前仍是**实验性**的，规则本身还在改）。

这也解释了为什么标准库把 `Pin` 做得这么小：**每一条 API 都是要背义务的**。
能让你用 `Deref` 解决的，就绝不给你 `DerefMut`。

## 19.7 小结

- **自引用在运行时就是一条 `stp`**：`stp x19, x0, [x0]` ——
  把自己的地址写进自己。没有运行时登记，没有魔法。
- **`async` 状态机真的会自引用**：MIR 的 `coroutine layout` 里
  `field _s0: String` 和 `field _s1: &String` 同时活在 `Suspend0` 变体里。
  这就是 `Pin` 最核心的用途：维护地址敏感值的不移动不变量。
- **`Pin` 靠收缩权限来表达"不能移动"**：拿走所有权（`E0507`）
  和 `&mut`（`E0596`）。错误信息说的是"cannot borrow as mutable"，
  而不是"这个类型不能移动"。
- **`Pin` 本身零成本**：`@plain = ... alias ... ptr @pinned`，
  汇编里是同一个符号。有开销的是 `Box::pin`（一次堆分配，换地址稳定）。
- **当前编译器生成的 `async` 产物不会自动实现 `Unpin`**，
  连没有 `await` 的 `async fn` 也是。应依赖这个可观察的 API 行为，
  不把它归因于未经本章证实的编译器 pass 时序。
- **★ 最反直觉的一条**：`addr_of!` 与 `addr_of_mut!` 生成**同样的汇编**，
  但前者在写入后会失效（SharedReadOnly），后者不会（Unique）。
  在本章这组后续写入中，一个指针仍有效、另一个已失效；Miri 能观察
  这种机器码中不存在的访问历史。
- **`Pin` 不保证值不被移动，只保证你拿不到移动它的手段**；
  **`!Unpin` 也不等于危险**，只是"移动需要额外证明"——
  与第 12 章的 `!Send` / `!Sync` 同一种语气。

第 18 章说"状态机不会自己跑，必须由 executor 驱动"；
本章说"状态机不能随便移动，所以必须被钉住"。
两句话合起来就是 `poll(self: Pin<&mut Self>)` 这个签名的全部来历。

下一章把这两条推到一起：**跨 `await` 的借用**如何影响 `Send`，
以及为什么一个 `MutexGuard` 会在 `await` 处把整个 future 变成 `!Send`。
