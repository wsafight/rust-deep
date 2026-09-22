# 12. Send 与 Sync 的真相

> 一句话：`Send` / `Sync` 是 **auto trait**，检查发生在**类型层**，
> 在单态化时被完全擦除。它们不是"运行时检查"，甚至不是"运行时概念"——
> **它们是编译期的记账，代价为零。**

## 先把语法认清

`Send` 表示值的所有权可以安全转移到另一线程，`Sync` 表示 `&T` 可以安全
在线程间共享；等价地，`T: Sync` 意味着 `&T: Send`。二者通常由字段自动
推导，手写 `unsafe impl Send/Sync` 等于替编译器承担完整线程安全证明。

把它们看成两枚通行章：`Send` 允许**整箱搬去另一条线程**，`Sync` 允许
**多人隔着共享引用同时看**。`Arc` 只负责箱子有几把钥匙，不会替里面的东西
补办通行证。

### 放到业务里：把请求任务提交到线程池

Web 服务把闭包交给 worker 时，闭包捕获的所有状态都必须 `Send`；多个 worker
共享同一配置时，配置还必须 `Sync`。`Rc` 不能跨线程，`Arc` 只解决共享所有权，
不会自动让内部的 `Cell` 线程安全。编译错误通常会沿字段链指出真正阻塞
`Send`/`Sync` 的那个成员。

```rust
let config = Arc::new(load_config());
std::thread::spawn({
    let config = Arc::clone(&config);
    move || serve(config)
});
```

闭包整体要 `Send`，而多个线程都能解引用 `Arc<Config>` 又要求 `Config: Sync`。

## 12.0 一个会让你卡住的例子

你想把一个 `Rc` 送进另一个线程：

```rust
use std::rc::Rc;

let r = Rc::new(42u64);
let h = std::thread::spawn(move || *r);      // ← 编译不过
```

```text
error[E0277]: `Rc<u64>` cannot be sent between threads safely
   |
 5 |     let h = std::thread::spawn(move || *r);
   |             ------------------ ^^^^^^^^^^ `Rc<u64>` cannot be sent between threads safely
   |
   = help: within `{closure@...}`, the trait `Send` is not implemented for `Rc<u64>`
note: required by a bound in `spawn`
```

换个写法，用 `Cell` 想跨线程共享：

```rust
use std::cell::Cell;

let c = Cell::new(42u64);
let r = &c;
std::thread::spawn(move || r.set(1));                // ← 也编译不过
```

```text
error[E0277]: `Cell<u64>` cannot be shared between threads safely
   |
   = help: the trait `Sync` is not implemented for `Cell<u64>`
   = note: if you want to do aliasing and mutation between multiple threads,
           use `std::sync::RwLock` or `std::sync::atomic::AtomicU64` instead
   = note: required for `&Cell<u64>` to implement `Send`
```

**注意最后一行** —— 编译器要检查的不是 `Cell<u64>`，而是 **`&Cell<u64>`**。
这一行就是 `Sync` 的定义（见 12.2.3）。

两段都过不了，但**报的是不同的 trait**（`Send` vs `Sync`）。

而下面这段**却能编译通过**：

```rust
let c = Cell::new(42u64);
let h = std::thread::spawn(move || c.get());     // ✅ Cell 是 Send！
```

**同一个 `Cell`，一次说"不能跨线程"，一次又能跨线程。**
这一章要把这个不对称说清楚。

## 12.1 先把常见说法摆上桌

通常会这样概括：

- `Send`：类型的所有权可以**转移**到另一个线程；
- `Sync`：类型的**共享引用**（`&T`）可以转移到另一个线程；
- 它们都是 **auto trait**：编译器自动为满足条件的类型实现；
- 大部分类型自动是 `Send + Sync`，例外是 `Rc`、`Cell`、裸指针等。

这些都对，但"可以转移"这个说法太模糊 —— 它没说清**检查发生在什么时候**，
也没解释 12.0 里那个不对称。

本章要建立的是两个更精确的直觉：

1. **检查发生在类型层，运行期零成本**（这是可以验证的）；
2. **`&T: Send` 当且仅当 `T: Sync`** —— 这才是 `Sync` 的真正定义。

## 12.2 编译器眼里的样子

### 12.2.1 汇编里**没有**任何"检查 `Send`"的代码

这是本章的第一条硬证据。实测：

```bash
grep -c 'Send\|Sync' .evidence/ch12-send-sync-lib.O3.s
# → 18
```

看起来有 18 处？但逐条看下去，它们**全部**在 mangled 符号名内部：

```text
__RINvNtCs8Mbv00yxnRz_4core3ptr9drop_glueINtNtB4_4cell10UnsafeCellINtNtB4_6option6OptionINtNtB4_6result6ResultjINtNtCshxvaOLs88l5_5alloc5boxed3BoxDNtNtB4_3any3AnyNtNtB4_6marker4SendEL_EEEEECsrtIYgyWToU_3lib:
```

把 `SendEL_` 那段解出来，它说的是
`Box<dyn Any + Send>` —— 是 std 内部 panic 机制的 drop glue 里的类型名。

**没有任何一处是"检查这个值能不能跨线程"的指令。**

对照 `local_only` 的完整函数体（一个既用 `Cell` 又用 `Rc` 的函数）：

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

**8 条指令，没有一条和线程安全有关。**
`Cell` 的"非同步"性质和 `Rc` 的"非原子引用计数"在这里都不产生任何额外代码 ——
因为**这个函数只在单线程里用**，而这件事已经由类型系统保证了。

> **`Send` / `Sync` 不是"运行时检查"，甚至不是"运行时概念"。**
> 它们是编译期的记账：编译器在类型层算清楚"谁能在哪个线程用"，
> 算完之后就把答案**丢掉**——生成的代码里什么都没有。

### 12.2.2 反证：不满足 `Send` 的程序**根本没有汇编**

上面说的是"检查不留痕迹"。更直接的证明是：
**编译失败的程序，没有汇编可看。**

```bash
rustc --edition 2024 --crate-type=lib examples/ch12-send-sync/fail/not_send.rs
```

```text
error[E0277]: `Rc<u64>` cannot be sent between threads safely
   --> examples/ch12-send-sync/fail/not_send.rs:7:24
    |
  7 |     std::thread::spawn(move || {
    |                       ^^^^^^^^ `Rc<u64>` cannot be sent between threads safely
```

**检查在编译期，不在运行期。** 这就是"零成本抽象"在并发上的体现：
不用 `Rc` 的线程安全是"运行时拒绝"，Rust 的是"编译期拒绝"。

### 12.2.3 `Sync` 的真正定义：`&T: Send`

回到 12.0 那个不对称。为什么 `Cell<u64>` 一次不行一次行？

`fail/not_sync.rs` 的错误信息里有一行**关键**的 note：

```text
   = note: required for `&Cell<u64>` to implement `Send`
```

★ **编译器解释得很清楚**：它要检查的不是 `Cell<u64>`，而是 **`&Cell<u64>`**。

标准库对 `Sync` 的定义就是这个：

```rust
impl<T: ?Sized + Sync> Send for &T {}
```

> **`Sync` 不是"另一个独立的性质"，它就是"`&T` 是 `Send`"这件事的名字。**

于是两个 auto trait 的分工就清楚了：

| | 含义 | 检查的是 |
|---|---|---|
| `T: Send` | 值本身能**搬**过去 | `T` |
| `T: Sync` | **共享引用**能搬过去 | `&T` |

`Cell<u64>` 是 `Send`（值可以整个搬走），但**不是** `Sync`
（`&Cell` 搬过去之后两个线程能同时写）。这个不对称**不是矛盾**，
是两个不同的性质。

### 12.2.4 ★★ 两个性质**互相独立**：四个格子全填满

`Cell` 是"`Send` 但 `!Sync`"。那有没有反过来的 ——
"`!Send` 但 `Sync`"？

有。`MutexGuard<'_, T>`：

```rust
pub fn guard_is_sync(g: &MutexGuard<'_, u64>) -> u64 { **g }   // ✅ 编译通过

let g = m.lock().unwrap();
std::thread::spawn(move || { let _ = *g; });                    // ❌ E0277
```

```text
error[E0277]: `std::sync::MutexGuard<'_, u64>` cannot be sent between threads safely
   = help: the trait `Send` is not implemented for `std::sync::MutexGuard<'_, u64>`
```

★ **为什么 `MutexGuard` 是 `!Send`？**
因为 pthread 只保证"**加锁的那个线程**能解锁"。
把 guard 移到别的线程去 drop（= unlock）是未定义行为。
标准库因此**只**给它实现了 `Sync`，**没有**实现 `Send`。

于是四张牌全都齐了：

| 类型 | `Send` | `Sync` | 证据 |
|---|---|---|---|
| `Cell<u64>` | ✅ | ❌ | `cell_is_send` / `fail/not_sync.rs` |
| `MutexGuard<'_, u64>` | ❌ | ✅ | `fail/guard_not_send.rs` / `guard_is_sync` |
| `Rc<u64>` | ❌ | ❌ | `fail/not_send.rs` |
| `u64` | ✅ | ✅ | — |

**四个格子全部填满 → 两个 trait 之间没有任何蕴含关系。**
"`Send` 和 `Sync` 有什么关系"这个问题的答案是：**没有关系**。

★ 这也解释了 `Arc<T>: Send` 为什么要求 `T: Send + Sync`：
`Arc` 既让你**移动**它（要 `Send`），也让你**共享**它（要 `Sync`）。
实测 `Arc<Cell<u64>>`：

```text
error[E0277]: `Cell<u64>` cannot be shared between threads safely
   = help: the trait `Sync` is not implemented for `Cell<u64>`
   = note: required for `Arc<Cell<u64>>` to implement `Send`
```

**两个要求都要满足 —— 缺一个都不行。**

★ 顺带看错误信息里那句建议：

```text
= note: if you want to do aliasing and mutation between multiple threads,
        use `std::sync::RwLock` or `std::sync::atomic::AtomicU64` instead
```

编译器不只是拒绝，还给出了正确的替代品。**"共享可变"不是不能做，
是要用对工具。**

### 12.2.5 ★ 实测踩到的坑：`unsafe impl Send` 只管**它标在哪个类型上**

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

明明 `MyBox: Send` 了，为什么还不行？

**因为闭包捕获的不是 `MyBox`，是 `b.0`。**

edition 2021 起，闭包做**精确捕获**（disjoint capture）：
`move || b.0 as usize` 只捕获用到的那个字段，
而 `b.0` 的类型是 `*mut u64` —— **不是 `Send`**。
于是 `unsafe impl Send for MyBox` **根本没被用上**。

**修法**：强制捕获整个结构体。

```rust
let h = std::thread::spawn(move || {
    let _ = &b;        // ← 强制捕获整个 MyBox
    b.0 as usize
});
```

> **深水洞察**：
> **`Send` 是"某个类型"的标记，不是"某段内存"的标记。**
> 标记加在 `MyBox` 上，就**只管** `MyBox` 本身。
> 捕获粒度变细之后，类型层的标记也跟着变细了 ——
> 这是精确捕获的一个容易被忽略的**副作用**。

### 12.2.6 `PhantomData` 决定 auto trait

同一个机制的另一面：**`PhantomData` 里写什么，决定了整个类型是不是 `Send`。**

```rust
pub struct Handle<T> {
    id: u64,
    _p: PhantomData<*const T>,      // ← 让 Handle<T> 变成 !Send + !Sync
}
```

实测的错误信息（`fail/phantom_not_send.rs`）：

```text
error[E0277]: `*const u64` cannot be sent between threads safely
   |
   = help: within `{closure@...}`, the trait `Send` is not implemented for `*const u64`
note: required because it appears within the type `PhantomData<*const u64>`
   |
811 | pub struct PhantomData<T: PointeeSized>;
```

★ **最后一行的措辞值得逐字读**：编译器指的是
`PhantomData<*const u64>`，**不是 `Handle<u64>`**。
它直接把"谁该为 `!Send` 负责"点了出来。

换一个写法就完全相反：

| `PhantomData<...>` | `Send`? | `Sync`? |
|---|---|---|
| `PhantomData<*const T>` | ❌ | ❌ |
| `PhantomData<fn() -> T>` | ✅ | ✅ |
| `PhantomData<T>` | 跟随 `T` | 跟随 `T` |

**因为 `fn() -> T` 是函数指针，而函数指针总是 `Send + Sync`。**

实测对照：`SafeHandle<T>`（用 `PhantomData<fn() -> T>`）的 `spawn_safe_handle`
编译通过；`Handle<T>`（用 `PhantomData<*const T>`）不行 ——
**同一个 `T`，只差 `PhantomData` 的写法。**

> 这是"用 `PhantomData` 精确控制 auto trait"的标准手法。
> 手写 `unsafe` 容器时（第 24–25 章），它是**唯一**的工具。

### 12.2.7 精确捕获的正面用法

同一个机制，在 `Handle` 上反而是**好事**：

```rust
pub struct Handle<T> { id: u64, _p: PhantomData<*const T> }   // 整体 !Send

pub fn spawn_handle_field(h: Handle<u64>) -> u64 {
    let h2 = std::thread::spawn(move || h.id);   // ← 只捕获 id（u64）
    h2.join().unwrap()
}
```

**编译通过** —— 因为闭包只捕获了 `id`，而 `u64` 是 `Send`。
`_p` 没被捕获，所以它带来的 `!Send` 也就没参与检查。

★ 对照 `spawn_mybox` 那个坑：

| 写法 | 捕获了什么 | 结果 |
|---|---|---|
| `move \|\| h.id` | 只有 `id: u64` | ✅ |
| `move \|\| { let _ = &h; h.id }` | 整个 `Handle<u64>` | ❌ E0277 |

**同一个机制的两面**：捕获粒度决定了**哪个类型**被拿去检查 auto trait。

> 这既是坑（`unsafe impl Send` 被绕过），也是工具
> （可以只捕获安全的那部分字段）。

## 12.3 为什么必须这样设计

### 为什么是 auto trait，而不是显式实现

如果 `Send` 要手动实现，那么：

- 每个结构体都要写 `unsafe impl Send for X {}`，**而 99% 的类型本来就应该安全**；
- 一旦漏写，代码就用不了 —— 而且是**假阴性**（本来安全的类型被拒绝）。

**auto trait 的语义是"结构性地自动推导"**：

```text
T: Send  ⟺  T 的所有字段都是 Send
```

这条规则让"安全"成为**默认值**，"不安全"才是要显式声明的例外。
这与 Rust 的整体哲学一致：**安全是默认的，`unsafe` 是要写出来的。**

### 为什么 `Sync` 要用 `&T: Send` 来定义

因为"能被多个线程同时持有"这件事，在 Rust 里**就是**"`&T` 能跨线程"。

Rust 没有"共享"这个原语 —— 共享总是通过引用（或 `Arc`，但 `Arc` 内部也是引用）。
所以"共享安全"必须定义为"**引用的转移安全**"：

```rust
impl<T: ?Sized + Sync> Send for &T {}
```

这条 `impl` 是 `Sync` 的**全部含义**。它是标准库里的实际代码。

### 为什么精确捕获让这件事变复杂

edition 2021 的精确捕获是个**纯粹的改进**（减少不必要的借用、让更多代码通过），
但它有一个副作用：**闭包的"类型"取决于它捕获了哪些字段**。

以前 `move || b.0` 捕获整个 `b`，`unsafe impl Send for MyBox` 就能救它；
现在只捕获 `b.0`，那个 impl 就够不着了。

**这不是 bug，是"标记粒度"和"捕获粒度"必须匹配的自然结果。**

## 12.4 反直觉的点

### 反直觉之一：`Cell<T>` 是 `Send`，只是不是 `Sync`

12.0 里那个"一次行一次不行"的例子，答案就是这一条。

```rust
let c = Cell::new(42u64);
std::thread::spawn(move || c.get());                 // ✅ Cell: Send

let c2 = Cell::new(42u64);
let r = &c2;
std::thread::spawn(move || r.set(1));                // ❌ &Cell: !Send
```

- **移动**一个 `Cell` 到另一个线程 → 此时只有一个线程能碰到它 → 安全；
- **共享**一个 `&Cell` 到另一个线程 → 两个线程能同时写 → 不安全。

**`!Sync` 不是"这个类型有毒"，是"它只在一个线程里安全"。**
`local_only` 那个函数就是证明：`Cell` 和 `Rc` 在单线程里**完全正常**。

### 反直觉之二：`unsafe impl Send` 可能"没生效"

`spawn_mybox` 那个坑：你明明写了 `unsafe impl Send for MyBox`，
编译器却说 `*mut u64` 不是 `Send`。

**因为标记加在 `MyBox` 上，而闭包捕获的是 `MyBox` 的字段。**
这不是编译器"没看见"你的 impl，是**那个 impl 真的和这次检查无关**。

> 教训：`unsafe impl Send` 是一个**类型级**的承诺。
> 它不会"渗透"到字段，也不会"保护"字段。

### 反直觉之三：`Send` / `Sync` 在汇编里**一个字都不剩**

`local_only` 的汇编只有 8 条指令，没有一条和线程安全有关。
`grep Send` 命中的 18 处**全部**在 mangled 符号名里。

**"线程安全"在 Rust 里不是一个运行时属性，是一个编译期属性。**
这和 `Arc` 的 `ldadd` 形成鲜明对照（第 13 章）：
`Arc` 的线程安全**真的**落在指令上，而 `Send` / `Sync` 什么都没留下。

**为什么？** 因为 `Arc` 的原子操作是**运行时**要执行的动作，
而 `Send` / `Sync` 是**编译期**的判断 —— 判断完了就不需要再执行了。

### 反直觉之四：`PhantomData` 是 auto trait 的开关

`PhantomData` 通常被理解成"占位符，让编译器以为我用了 `T`"。
但在 auto trait 的语境里，它是**控制器**：

```rust
PhantomData<*const T>      // → !Send + !Sync
PhantomData<fn() -> T>     // → Send + Sync
PhantomData<T>             // → 跟随 T
PhantomData<&'a T>         // → 跟随 T
```

**同一个 `T`，三种不同的 auto trait 组合。**
这是手写 `unsafe` 容器时唯一的精细控制手段。

## 12.5 亲手验证

```bash
tools/evidence.sh ch12-send-sync
scripts/verify-all.sh ch12

# ★ 汇编里没有"检查 Send"的代码（命中的全在符号名里）
grep -o 'Send[A-Za-z0-9_]*' .evidence/ch12-send-sync-lib.O3.s | sort -u

# 单线程里 Cell/Rc 完全正常（8 条指令，无同步开销）
awk '/^_local_only:/,/cfi_endproc/' .evidence/ch12-send-sync-lib.O3.s

# 三个反例
for f in not_send not_sync phantom_not_send; do
  rustc --edition 2024 --crate-type=lib examples/ch12-send-sync/fail/$f.rs 2>&1 | head -3
done
```

**怎么算验证成功**：

1. `grep 'Send'` 在 `.O3.s` 里命中的内容**全部**在 mangled 符号名内部
   （形如 `...marker4SendEL_...`），**没有独立的指令**；
2. `local_only` 的函数体只有 8 条指令，没有任何同步原语；
3. `fail/not_send.rs` 报 **E0277**（`Rc` 不是 `Send`）；
4. `fail/not_sync.rs` 报 **E0277**，且 note 里有
   `` required for `&Cell<u64>` to implement `Send` `` —— **`Sync` 的定义**；
5. `fail/phantom_not_send.rs` 的 note 指向
   `` required because it appears within the type `PhantomData<*const u64>` ``；
6. `fail/guard_not_send.rs` 报 **E0277**，而 `guard_is_sync` **编译通过**
   —— 这一对就是"`Sync` 与 `Send` 互相独立"的全部证据。

```bash
scripts/verify-all.sh ch12      # 9 条断言
```

## 12.6 与 unsafe 的关系

这一章和 `unsafe` 的关系是**全书最直接的之一**：

**`Send` / `Sync` 是 `unsafe` 能撒的两种谎。**

```rust
pub struct MyBox(pub *mut u64);
unsafe impl Send for MyBox {}      // ← 一个"谎"：我保证它跨线程安全
```

编译器**不会**验证这个承诺。它只会在你撒谎之后，把代码编译成
**假定你不会撒谎**的样子 —— 那就是 UB。

★ 本章的三个坑正好构成"撒谎的三种翻车方式"：

1. **`spawn_mybox`**：谎撒在 `MyBox` 上，但字段被单独检查 ——
   你以为保护了，其实没保护到；
2. **`PhantomData<*const T>`**：谎撒反了 —— 本来安全的类型被标成不安全
   （假阴性，不危险但很烦）；
3. **`PhantomData<fn() -> T>` 配真裸指针**：这是最危险的 ——
   用 `fn() -> T` 让类型**看起来**是 `Send`，而字段里真的有裸指针。
   **这是标准库之外的代码里最常见的 `unsafe` 谎言。**

> 第 24–25 章会展开：`unsafe impl Send` 的**正确论证**长什么样。
> 简版是：你必须证明"这个值被移到另一个线程之后，
> **没有任何两个线程能同时访问同一块内存，或者访问时有正确的同步**"。
> 这句话不是形式化的，但它是唯一的判据。

## 12.7 小结

- **`Send` / `Sync` 是 auto trait，检查在类型层，运行期零成本。**
  实测：`local_only`（用了 `Cell` 和 `Rc`）的汇编只有 8 条指令，
  没有任何同步代码；`grep Send` 命中的全在 mangled 符号名里。
- **`Sync` 的定义就是 `&T: Send`**（`impl<T: Sync> Send for &T {}`）。
  所以"能不能共享"这个问题，编译器是用 `Send` 来回答 `&T` 的。
- **`Cell<T>` 是 `Send` 但 `!Sync`；`MutexGuard` 是 `Sync` 但 `!Send`。**
  四张牌（`Send`×`Sync` 的四种组合）全部齐了 ——
  **两个 trait 之间没有任何蕴含关系。**
- **`Arc<T>: Send` 要求 `T: Send + Sync`**：`Arc` 既能被移动，也能被共享。
- **`unsafe impl Send for MyBox` 只管 `MyBox`，不管它的字段。**
  edition 2021 的精确捕获会让闭包捕获单个字段，从而**绕过**这个 impl。
  修法是 `let _ = &b;` 强制捕获整个结构体。
- **`PhantomData` 是 auto trait 的开关**：
  `*const T` → `!Send`，`fn() -> T` → `Send`，`T` → 跟随 `T`。
  手写 `unsafe` 容器时这是唯一的精细控制手段。
- **不满足 `Send` 的程序没有汇编** —— 检查在编译期，
  这就是"零成本抽象"在并发上的体现。

下一章把 `Send` / `Sync` 落到**指令**上：`Arc` 的引用计数、
`Mutex` 的平台差异，以及为什么"线程安全"在 `Arc` 那里是**真的**有代码的。
