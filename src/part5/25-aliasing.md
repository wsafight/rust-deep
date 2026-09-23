# 25. 别名规则、`UnsafeCell` 与 `PhantomData`

> 一句话：别名规则不只是"两条指针能不能重叠"。在 Miri 默认采用的
> Stacked Borrows 模型里，可以把每块内存想成带着一组访问权限。
> 模型关心**指针的出处和访问历史**，不只看地址是否相同 ——
> 所以**同一个逻辑，写在不同的位置，一个是 sound 的，一个是 UB**。

## 先把语法认清

`UnsafeCell<T>` 是共享引用下合法修改数据的基础；`UnsafeCell::get()` 返回
裸指针，调用者仍需保证没有数据竞争和无效引用。`PhantomData<U>` 不占空间，
用于告诉类型系统外层类型在逻辑上拥有或借用了 `U`，从而影响生命周期、
drop check、variance 和 auto trait。

这两样东西常被放在一起讲，却干着不同的活：`UnsafeCell` 改变“能不能通过
共享引用写”，`PhantomData` 补充“这个外层类型在逻辑上和谁有关系”。

### 放到业务里：自定义容器、缓存与 FFI 句柄

实现 arena、对象池或 FFI handle 时，结构体可能只保存裸指针，却在语义上
借用某块内存；这时需要正确的 `PhantomData`。实现锁、Cell 或缓存内部更新时，
则必须通过 `UnsafeCell` 表达内部可变性。两者职责不同：前者补静态类型关系，
后者改变共享引用下的可变性规则。

```rust,ignore
struct Handle<'a> {
    ptr: *mut Record,
    _borrow: PhantomData<&'a mut Record>,
}
```

这个 ZST 字段不会修复 `ptr`，却会让 `Handle` 的生命周期和 variance 更接近
“持有一份可变借用”的真实语义；裸指针有效性仍由构造器与使用者负责。

第 24 章讲清了 `unsafe` 的义务是维护安全抽象依赖的不变量，
LLVM 元数据只是其中可观察的一层。
本章要看的是最难维持的那一条：**别名**。

而且本章有一个特殊的地位：**它的主证据不是编译器输出，是 Miri。**
先解释为什么。

## 25.0 一个会让你卡住的例子

你想写一个"共享的计数器"：

```rust,ignore
pub struct BadCounter { n: u64 }

impl BadCounter {
    pub fn bump(&self) {                 // ← 注意：&self，不是 &mut self
        let p = &self.n as *const u64 as *mut u64;
        unsafe { *p += 1 }
    }
}
```

**这段代码编译不过** —— 但不是因为借用检查器：

```text
error: assigning to `&T` is undefined behavior, consider using an `UnsafeCell`
  --> ...
   |
30 |     let p = x as *const u64 as *mut u64;
   |             --------------------------- casting happened here
32 |     unsafe { *p = v }
   |              ^^^^^^
   = note: `#[deny(invalid_reference_casting)]` on by default
```

**注意这条错误的措辞**：它说的不是"你不能写 `&T`"，而是
**"这是 UB，考虑用 `UnsafeCell`"**。而且这个 lint 是 **`deny` by default**。

于是你改用 `UnsafeCell`：

```rust,ignore
pub struct Cell2<T> { inner: UnsafeCell<T> }

impl<T: Copy> Cell2<T> {
    pub fn set(&self, v: T) {
        unsafe { *self.inner.get() = v }   // ← 同样是"通过共享引用写"，这次编译过了
    }
}
```

**编译过了。** 区别在哪？为什么 `UnsafeCell` 可以？

更奇怪的是第三个问题。下面这段**编译得过、也跑得对**，但 Miri 说它是 UB：

```rust,ignore
let c = UnsafeCell::new(1u64);
let r: &u64 = unsafe { &*c.get() };   // ① 先建一个 &T
unsafe { *c.get() = 2 };              // ② 通过 UnsafeCell 写
assert_eq!(*r, 2);                    // ③ 再用 ① 那个 &T
```

```text
error: Undefined Behavior: trying to retag from <N> for SharedReadOnly permission
       at alloc<N>[0x0], but that tag does not exist in the borrow stack
  = help: <N> was created by a SharedReadOnly retag at offsets [0x0..0x8]
  = help: <N> was later invalidated at offsets [0x0..0x8] by a write access
```

**把 ① 和 ② 的顺序换一下**，同样三行代码，Miri 就通过了。

本章要回答的就是这三个问题。而它们的答案指向同一个东西：
**别名规则管的是权限的出处，不是"谁和谁重叠"。**

## 25.1 先把常见说法摆上桌

通常会这样概括：

- Rust 的别名规则是"同一时刻要么一个 `&mut`，要么任意多个 `&`"；
- `UnsafeCell` 是内部可变性的基础，`Cell` / `RefCell` / `Mutex` 都建在它上面；
- `PhantomData` 用来标记类型参数，影响 auto trait 和 variance；
- 违反别名规则是 UB。

这些都对，但**它们描述的规则无法解释 25.0 的三个现象**：

- 为什么同样的"通过共享引用写"，一处报错一处不报？
- 为什么"先用 `&T` 再写"是 UB，而"先写再用 `&T`"不是？
- 如果规则是"不能同时有 `&` 和写"，那 `UnsafeCell` 为什么是例外？

要回答这些，得换一个模型。

## 25.2 编译器眼里的样子

### 25.2.1 先说清楚：为什么本章的证据来自 Miri

⚠️ **这是一个必须讲明白的坑。**

第 1–24 章的 MIR 证据都很顺手：`--emit=mir` 一打，借用、生命周期、
状态机布局全都看得见。但**别名规则在 MIR 里看不见**。

原因是 **retag 不在 MIR 里**：

| 写法 | `--emit=mir` 里出现什么 |
|---|---|
| `pub fn m(a: &mut u64)`（顶层引用参数） | ❌ 什么都不出现 |
| `pub fn f(a: &mut u64, b: &mut u64)` | ❌ 什么都不出现 |
| `let p = (a,); let q = black_box(p); **q.0` | ✅ `no_retag` |
| `&Box<u64>` 解引用、`&mut &mut u64` 重借用 | ✅ `no_retag` |

**看起来"有 retag 的地方反而不打印"，很反直觉。** 机制（读 rustc 1.98 源码确认）：

1. `Rvalue::Use` 带一个 `WithRetag::Yes/No` 标志
   （`rustc_middle/src/mir/syntax.rs`）；
2. pretty printer **只在 `No` 时**打印 `no_retag`
   （`rustc_middle/src/mir/pretty.rs`，源码注释原文：
   *"With retag is more common so we only print when it's without."*）。

所以：

> **`--emit=mir` 里的 `no_retag` = "这里故意不做 retag"。**
> 而**做了 retag 的地方是静默的** —— 它根本不打印。

stable 上 `no_retag` 只有两个来源：`EraseDerefTemps`
（把 `CopyForDeref` 改写成 `Use(.., WithRetag::No)`，源码注释
*"We do **NOT** want a retag here!"*）和 `DerefSeparator`。

**更关键的一条**：`-Zdump-mir`（逐 pass dump）**也看不到 retag** ——
1.98 的 pass 列表里根本没有 `AddRetag`。**retag 是 codegen 阶段的事**。

（要看 retag 真的生成了代码，用 `-Zcodegen-emit-retag`（需 nightly）——
汇编里会出现 `bl ___rust_retag_reg`。）

**所以本章的证据层是：**

| 层 | 工具 | 承担什么 |
|---|---|---|
| **主** | **Miri** | 按当前 Stacked Borrows 模型动态检查具体执行路径 |
| 辅 | LLVM IR | 契约（`noalias` / `captures`）—— 见第 24 章 |
| 澄清 | MIR | 只用来**破除误解**：`no_retag` 不是 retag |

MIR 在本章只能承担"展示 `no_retag` 长什么样、纠正误解"的角色。
**撑不起"别名规则"的主证据。**

### 25.2.2 一个实用但仍在演进的模型：Stacked Borrows

本章采用的 Stacked Borrows 模型不只问“两条指针是否重叠”，而是：

> **每块内存有一个权限栈。指针的"出处"决定了它带什么权限。
> 每次访问都会调整这个栈 —— 用它，就把它**之上**的权限弹掉。**

于是 25.0 的三个现象都有了解释：

| 现象 | 解释 |
|---|---|
| `&T` 转 `*mut T` 写 → 报错 | `&T` 派生的是 **SharedReadOnly** 权限；写操作**根本不是它允许的** |
| `UnsafeCell` 可以 | `&UnsafeCell<T>` 派生的权限是 **SharedReadWrite** —— 写是允许的 |
| 先建 `&T` 再写 → UB | 那次写把 `&T` 的 SharedReadOnly **弹掉了** |

**规则管的是权限，不是"谁和谁重叠"。** 这是本章要建立的核心直觉。

### 25.2.3 `UnsafeCell` 是唯一合法的例外

`UnsafeCell` 的全部作用就是：**把"只读"这条性质从 `&UnsafeCell<T>` 上摘掉。**

```rust,ignore
pub struct Cell2<T> { inner: UnsafeCell<T> }

impl<T: Copy> Cell2<T> {
    pub fn get(&self) -> T {
        // SAFETY: UnsafeCell 保证通过共享引用修改是允许的
        unsafe { *self.inner.get() }
    }
    pub fn set(&self, v: T) {
        // SAFETY: 同上
        unsafe { *self.inner.get() = v }
    }
}
```

它是**标准库里唯一一个**能做这件事的类型 ——
`Cell`、`RefCell`、`Mutex`、`RwLock`、`AtomicUsize`……
**全部**建在它上面。

★ 编译器的错误信息**直接给出了这个答案**：

```text
error: assigning to `&T` is undefined behavior, consider using an `UnsafeCell`
```

**rustc 不只是拒绝你，它告诉了你正确做法。** 这个 lint
（`invalid_reference_casting`）是 `deny` by default。

### 25.2.4 顺序决定一切

25.0 的第三个现象是本章最微妙的。实测（Miri）三种顺序：

| 顺序 | 结果 |
|---|---|
| ① 建 `&T` → ② 通过 `UnsafeCell` 写 → ③ 用 `&T` | ❌ **UB** |
| ② 通过 `UnsafeCell` 写 → ① 建 `&T` | ✅ 合法 |
| 全程只用 `UnsafeCell` 指针，从不建 `&T` | ✅ 合法 |

**`UnsafeCell` 让"那次写入"合法，但这次写入仍然会弹掉在此之前建立的 `&T`。**

> **实践规则：不要在写入之前建立 `&T` 并持有到写入之后。**

对照的两个函数**只差两行的顺序**：

```rust,ignore
// UB（tests/aliasing_ub.rs）
let r: &u64 = unsafe { &*c.get() };      // ① 先建 &T
w.set(2);                                 // ② 再写
assert_eq!(*r, 2);                        // ③ 用 → UB

// 合法（tests/aliasing.rs）
w.set(2);                                 // ① 先写
let r: &u64 = unsafe { &*c.get() };      // ② 后建 &T
assert_eq!(*r, 2);                        // ✅
```

### 25.2.5 `PhantomData`：补回类型系统看不见的关系

裸指针在 Rust 的**类型检查**层面不表达"这个结构体借用了谁、拥有谁"。
但在 Miri 的动态别名模型里，裸指针仍有 provenance/tag，它来自指针的
实际创建路径，**不是**由 `PhantomData` 赋予的。

`PhantomData` 的职责，是把所有权或借用关系补回静态类型系统：

```rust,ignore
pub struct SharedReadOnly<'a, T> {
    ptr: *const T,
    _p: PhantomData<&'a T>,                  // ← 类型层面借用了 &'a T
}

pub struct SharedReadWrite<'a, T> {
    ptr: *mut T,
    _p: PhantomData<&'a UnsafeCell<T>>,      // ← 类型层面借用了 UnsafeCell<T>
}
```

**两个结构体的字段布局完全一样**（8 字节指针 + ZST）。
二者访问权限不同，是因为构造器里的真实指针分别来自 `&T` 和
`UnsafeCell::get()`；即使删掉 `PhantomData`，Miri 也不会因此改变那个指针
已经具有的 provenance。`PhantomData` 改变的是编译期关系，而不是运行时 tag。

`PhantomData` 是零大小的，但它让类型**参与**三件事：

1. **生命周期检查** —— 这个指针借用了 `'a`，`'a` 内不能动那个值；
2. **auto trait 推导** —— 第 12 章：`PhantomData<*const T>` 让类型 `!Send`；
3. **variance** —— 第 3 章：`PhantomData<&'a T>` 是协变的。

★ 本章用的是第 1 条和第 3 条：**让拥有裸指针的外层类型表现得像它
借用了 `&'a T` 或 `&'a UnsafeCell<T>`**。Miri 判断一次访问是否合法时，
看的仍是实际被解引用指针的来源和访问历史。

**这就是 `PhantomData` 在 `unsafe` 代码里的真正用途** ——
不是给指针制造权限，而是把类型系统无法从裸指针字段推导出的
生命周期、所有权、variance 和 auto trait 关系显式写出来。

### 25.2.6 rustc 的 lint 兜得住什么、兜不住什么

`fail/write_through_shared_ref.rs` 里那个写法会被 rustc 直接拒绝。
但**把铸型点藏进一个函数、再用 `black_box` 挡住数据流**，
rustc 的 lint 就抓不到了（`tests/aliasing_ub.rs`）：

```rust,ignore
let r: &u64 = unsafe { &*c.get() };
let p = black_box(r as *const u64 as *mut u64);
unsafe { *p = 2 };      // ← rustc 不报，Miri 报
```

★ **同一段逻辑，写在一个函数里会被拒，拆成两个函数就编译通过。**

这正是第 24 章那句话的具体形态：
**`unsafe` 的义务不写在代码里** —— 而静态检查只能覆盖它能看见的部分。

## 25.3 为什么必须这样设计

### 为什么需要"权限栈"这么复杂的模型

因为"两条指针不能重叠"这个规则**太强了**，会禁掉大量合法的代码：

```rust,ignore
let v = 21u64;
let (a, b) = unsafe { (&*p, &*p) };   // 两个共享引用指向同一处 —— 完全合法
```

第 24 章已经说明：LLVM 的 `noalias` **允许**这件事
（LangRef："only holds for memory locations that are **modified**"）。

所以规则必须更精细：

- **共享引用可以重叠**（只要没人写）；
- **写入要"作废"之前建立的共享引用**（因为那些引用的持有者可能做了假设）；
- **`UnsafeCell` 是显式的例外**（"我知道这里有共享写，我是故意的"）。

**权限栈模型恰好表达这三条**：重叠的共享引用都在栈里，
写入把上面的弹掉，而 `UnsafeCell` 派生的权限是 SharedReadWrite（写不弹它）。

### 为什么 `UnsafeCell` 必须是"唯一"的例外

因为它把一个**类型层的事实**（"这个内存会被共享写"）变成了**可检查的**。

没有 `UnsafeCell` 的话，"通过共享引用写"就只是一个**约定** ——
编译器不知道，Miri 也不知道，出问题只能靠"碰巧能跑"。
有了它，编译器可以：

- 让 `&UnsafeCell<T>` **不是** `noalias`（第 24 章的元数据直接变了）；
- 让 Miri 知道这个指针的权限是 SharedReadWrite；
- 让 `invalid_reference_casting` lint 有明确的"正确做法"可以推荐。

★ 这也是为什么**所有内部可变性类型都必须经由 `UnsafeCell`** ——
绕过它（自己转裸指针）会让这些机制全部失效。

### 为什么 `PhantomData` 不是"占位符"

因为仅凭裸指针字段，类型系统无法推导 API 想表达的借用/所有权关系，
特别是生命周期、variance、drop check 与 auto trait 行为。

```rust,ignore
pub struct Handle { ptr: *mut u64 }                    // 丢了全部三样
pub struct Handle<'a> { ptr: *mut u64, _p: PhantomData<&'a u64> }   // 补回来了
```

**`PhantomData` 就是把这些静态关系补回来的工具。**
它可以让外层类型在 variance、drop check 和 auto trait 推导上表现得像
拥有或借用了某个 `T`，但不等价于真实引用，也不改变裸指针的 provenance。

所以当一个含裸指针的类型在逻辑上拥有或借用了某个值、而字段本身无法
表达这种关系时，`PhantomData` 往往是必须的；若没有这种语义关系，
则不能机械地认为每个裸指针字段都必须搭配它。

## 25.4 反直觉的点

### 反直觉之一：`no_retag` 是"不做 retag"的标记，不是 retag 的证据

25.2.1 已经展开。要特别避免把 `no_retag` 当成
"retag 在借用检查阶段被折叠"的证据；这种归因是错的。

正确的说法：

> `--emit=mir` 里的 `no_retag` 恰恰是"这里**不**做 retag"。
> 做了 retag 的地方**静默**（`WithRetag::Yes` 不打印任何东西）。
> 而且 retag 在 1.98 里是 **codegen 阶段**的事，
> 任何 MIR 打印（`--emit=mir` 或 `-Zdump-mir`）都看不到。

★ 这个教训值得单独记住：**"我在输出里看到了 X"不等于"X 在这里发生"。**
要确认一个机制在哪里发生，得读**源码**，而不是读输出。

### 反直觉之二：`UnsafeCell` 不会"救活"已经存在的 `&T`

直觉上会以为："既然 `UnsafeCell` 允许共享写，那从 `&T` 派生的指针应该一直有效。"

**不是。** 顺序决定一切（25.2.4）：

```rust,ignore
// ❌ UB：&T 建立在前
let r: &u64 = unsafe { &*c.get() };
unsafe { *c.get() = 2 };
assert_eq!(*r, 2);

// ✅ 合法：&T 建立在后
unsafe { *c.get() = 2 };
let r: &u64 = unsafe { &*c.get() };
assert_eq!(*r, 2);
```

**实践规则：需要共享读，就在最后一次写之后建引用。**

★ 这条对 `RefCell` 那类设计有直接影响：
`RefCell::borrow()` 返回的 `Ref<'_, T>` 是一个 `&T`（经由 `UnsafeCell` 派生），
在它活着的时候调 `borrow_mut()` 会被**运行时**借用检查拦住 ——
这是设计上的巧合还是必然？值得想一想。

### 反直觉之三：rustc 有一个专门的 lint，而且是 `deny`

```text
error: assigning to `&T` is undefined behavior, consider using an `UnsafeCell`
   |
30 |     let p = x as *const u64 as *mut u64;
   |             --------------------------- casting happened here
   = note: `#[deny(invalid_reference_casting)]` on by default
```

**注意它把铸型点和写入点都标出来了** ——
因为 UB 是这两件事**合起来**造成的。

★ 但它是**局部的**：只看得到同一个函数体内的铸型 → 写入链。
跨函数、或者经由 `black_box` 就抓不到了（25.2.6）。

**所以"编译过了"在这类问题上一文不值。**

### 反直觉之四：Miri 的规则**还是实验性的**

每次 Miri 报 UB，它都会附一句：

```text
= help: this indicates a potential bug in the program: it performed an invalid
        operation, but the Stacked Borrows rules it violated are still experimental
```

这不是客套话。**Stacked Borrows 至今仍是实验性的**，
另有一个竞争模型 **Tree Borrows**（`-Zmiri-tree-borrows` 可切换）。

★ 实践含义：
- Miri 报 UB → 说明代码违反当前所选模型，应结合语言规则与 API 契约认真核查；
- Miri 不报 → **不能反推"代码一定 sound"**（模型可能更严或更松）；
- **不要**把"Miri 通过"当成 soundness 证明写在文档里。

### 反直觉之五：`&T` 的只读性是**语义**，不是"编译器暂时没优化"

一个常见误解："`&T` 不能写只是因为编译器不让你写，转了裸指针就行了。"

**不对。** `&T` 的只读性是**语义层的事实**：
LLVM 拿到的 `noalias` + `readonly`（第 24 章）就是**建立在这个事实之上**的。
你通过裸指针写它，不是"绕过了一个检查"，而是**让 LLVM 已有的假设变成了谎言**。

这就是为什么 rustc 会说"**这是 UB**"，而不是"你不能这么做"。

## 25.5 亲手验证

```bash
# ★ 本章的主证据：Miri（需要 nightly，见附录 A）
scripts/verify-miri.sh      # 7 条：ch25 四条 + ch19 两条 + 前置检查

# 辅：MIR 里的 no_retag（只用来破除误解）
tools/evidence.sh ch25-aliasing
grep no_retag .evidence/ch25-aliasing-lib.mir

# ★ rustc 的 lint 抓"通过 &T 写"
rustc --edition 2024 --crate-type=lib examples/ch25-aliasing/fail/write_through_shared_ref.rs
```

**怎么算验证成功**：

1. `scripts/verify-miri.sh` 里 **ch25 的四条全绿**：
   - `aliasing`（8 个用例）**通过** —— `UnsafeCell` / `PhantomData` 的合法用法；
   - `aliasing_ub`（4 个用例）**报 UB** —— 顺序反了、跨函数藏起铸型点；
   - `sb_legal` 通过、`sb_ub` 报 UB（第 24 章那个 LangRef 结论的运行期验证）；
2. `grep no_retag` 能看到 `_4 = no_retag copy (_3.0: &&u64)` ——
   **并理解它恰恰是"这里不做 retag"**；
3. `rustc` 拒绝 `fail/write_through_shared_ref.rs`，
   错误里含 `invalid_reference_casting` 和 `consider using an UnsafeCell`。

★ 自己动手验证 25.2.4 那条顺序规则，最快的办法是把
`tests/aliasing.rs` 的 `write_then_read_is_fine` 里两行**换个位置**，
然后跑 `cargo +nightly miri test -p ch25-aliasing --test aliasing` ——
**会从通过变成 UB。**

## 25.6 与 unsafe 的关系

本章整章都是 `unsafe` 的边界。收成三条**可操作**的规则：

### 规则一：想通过共享引用修改，用 `UnsafeCell`

不要自己转裸指针。rustc 会拒绝你，而且拒绝得有道理 ——
`UnsafeCell` 不只是一个"能过编译的写法"，
它让**编译器和 Miri 都知道**这里会发生共享写。

### 规则二：注意引用的建立顺序

`&T` 建立之后发生的写入会作废它。所以：

- **先写完，再建引用**；
- 如果一个结构体在逻辑上借用 `UnsafeCell<T>`，用恰当的
  `PhantomData<&'a UnsafeCell<T>>` 表达这层静态借用关系；
  这不能代替对真实裸指针来源和访问顺序的检查。

### 规则三：按真实语义选择 `PhantomData`

裸指针字段本身表达不了外层类型的借用/所有权语义。需要表达这些关系时，
用 `PhantomData` 影响生命周期、drop check、auto trait 与 variance；
补错通常表现为静态类型属性错误，不能指望 Miri 自动诊断。

```rust,ignore
// 若 Handle 在逻辑上借用 T，这里缺少生命周期关系
struct Handle { p: *mut T }

// 表达：Handle 在 'a 内共享借用了一个 UnsafeCell<T>
struct Handle<'a, T> { p: *mut T, _m: PhantomData<&'a UnsafeCell<T>> }
```

### 一个可以照着抄的检查清单

写含裸指针的 `unsafe` 代码时，逐条问：

1. 这个指针的**权限**是什么？（只读 / 可写 / 独占）
   —— 它由**出处**决定，不是由类型决定。
2. 有没有 `&T` 在写入之前建立、在写入之后还活着？
3. 裸指针字段配的 `PhantomData` 对不对？
4. `SAFETY` 注释里写清楚"我依赖哪条规则、为什么没破坏它"了吗？
5. **Miri 跑了吗？**（`cargo +nightly miri test`）

第 26 章会换个角度：**什么时候根本不该用 `unsafe`。**

## 25.7 小结

- **Stacked Borrows 提供了一个实用的“权限栈”模型**，比
  “指针不能重叠”更精细。它关心指针出处和访问历史，所以同一个逻辑写在不同位置，
  可能一个是 sound 的、一个是 UB。
- **`UnsafeCell` 是唯一合法的"通过共享引用修改"**：
  它把"只读"从 `&UnsafeCell<T>` 上摘掉。所有内部可变性类型都建在它上面。
- **顺序决定一切**：`UnsafeCell` 让那次写入合法，
  但**不会救活一个在此之前建立的 `&T`**。先写完，再建引用。
- **`PhantomData` 补的是静态类型关系**
  （生命周期 / drop check / auto trait / variance），不制造 Miri 权限，
  也不改变裸指针的 provenance。是否需要它取决于类型要表达的所有权和借用关系。
- **★ 本章的证据来自 Miri，不是 MIR**：
  retag 在 1.98 里是 **codegen 阶段**的事，任何 MIR 打印都看不到。
  `--emit=mir` 里的 `no_retag` 恰恰是"这里**不**做 retag"的标记。
- **rustc 的 `invalid_reference_casting` lint 是 `deny` 默认**，
  但它只能看见同一函数体内的铸型链 —— 跨函数就漏。
  **"编译过了"在这类问题上一文不值。**
- **Miri 的规则仍是实验性的**（Stacked Borrows vs Tree Borrows）：
  报 UB 要认真查，不报**不能**反推 sound。

下一章收尾：**什么时候根本不该用 `unsafe`** ——
给出一个可以照着走的判断流程，而不是一句"尽量别用"。
