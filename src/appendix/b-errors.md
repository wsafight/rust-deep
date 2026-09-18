# 附录 B：常见编译错误逐条解读

> 最后验证：**rustc 1.98.1**（2026-09-01）/ `aarch64-apple-darwin`

本附录按**错误码**索引本书正文里出现过的编译错误。
每一条都给出：**原文**、**在哪一章展开**、**修法**。

★ 全书所有错误原文都是**本机实测**的 ——
对应 `examples/*/fail/*.rs` 里的反例，并由 `scripts/verify-all.sh` 的
`assert_fails` 断言"必须报出这个错误码"。

---

## E0038：trait 不是 dyn compatible

```text
error[E0038]: the trait `Bad` is not dyn compatible
  = note: for a trait to be dyn compatible it needs to allow building a vtable
```

**含义**：这个 trait 建不出 vtable。

**为什么**：vtable 是一张**固定布局**的函数指针表（第 7 章：
前 3 个 slot 是 `drop` / `size` / `align`，方法从偏移 24 起）。
泛型方法会为每个类型参数产生**不同数量**的槽位，表就定不下大小；
返回 `Self` 同理（调用点不知道返回类型有多大）。

**修法**：

- 把泛型方法挪到另一个 trait（`where Self: Sized` 的方法不参与 vtable）；
- 或者改用泛型参数（单态化）；
- 或者返回 `Box<dyn Trait>`。

**章**：7（vtable）、10（GAT）、11（实战）。

**反例**：`ch07-vtable/fail/not_dyn_compatible_generic.rs`、
`ch07-vtable/fail/not_dyn_compatible_self.rs`、
`ch10-gat/fail/gat_not_dyn.rs`、`ch11-project-abstract/fail/generic_method_kills_dyn.rs`。

---

## E0106：缺少生命周期标注

```text
error[E0106]: missing lifetime specifier
  = help: this function's return type contains a borrowed value, but the
          signature does not say whether it is borrowed from `x` or `y`
```

**含义**：返回的引用不知道借自哪个输入。

**为什么**：省略规则（第 2 章）只处理三种情形 ——
单输入引用、`&self` / `&mut self`、无引用输入。
**两个输入引用 + 输出引用**推不出来。

**修法**：显式写 `<'a>`：

```rust
pub fn pick<'a>(x: &'a [u64], y: &'a [u64]) -> &'a u64 { ... }
```

★ **`async fn` 不改变这条规则**（第 20 章实测）。

**章**：2、20。

**反例**：`ch02-lifetimes/fail/missing_lifetime.rs`、
`ch20-async-lifetimes/fail/ambiguous_lifetime.rs`。

---

## E0117：孤儿规则 —— 外部 trait + 外部类型

```text
error[E0117]: only traits defined in the current crate can be implemented
              for types defined outside of the crate
```

**含义**：不能给"外部的类型"实现"外部的 trait"。

**为什么**：如果允许，两个 crate 可能给同一个类型实现同一个 trait
（coherence 会被破坏）。规则是"**trait 或类型至少有一个是本 crate 的**"。

**修法**：newtype 包装（`struct Wrapper(ExternalType);`），
或者在自己这边定义 trait。

**章**：8。

**反例**：`ch08-coherence/fail/orphan.rs`、`ch08-coherence/fail/orphan_vec.rs`。

★ `Vec<Local>` 仍然被拒 —— 因为 `Vec` 不是 `#[fundamental]`。
（`Box` 和 `&` 是。）

---

## E0119：实现冲突

```text
error[E0119]: conflicting implementations of trait `Container` for type `Numbers`
```

**含义**：同一个类型实现了同一个 trait 两次。

**为什么**：关联类型（`type Item`）是 `Self` 的**函数** ——
一个输入只能有一个输出（第 6 章）。

**修法**：改用泛型参数（`trait ContainerG<T>`），
同一个类型就能实现多次。

★ 另一个常见形态：**blanket impl 与具体 impl 冲突**
（`ch08-coherence/fail/conflicting_blanket.rs`）：

```text
error[E0119]: conflicting implementations of trait `MyTrait` for type `u64`
```

**章**：6、8、11。

---

## E0210：类型参数没被本地类型覆盖

```text
error[E0210]: type parameter `T` must be used as an argument to some local type
              (e.g., `MyStruct<T>`)
```

**含义**：这是孤儿规则的"泛型版"。
`impl<T> ExternalTrait for T` —— `T` 可能是任何类型，包括外部的。

**修法**：让 `T` 出现在本地类型的参数位置：
`impl<T> ExternalTrait for MyStruct<T>`。

**章**：8。

**反例**：`ch08-coherence/fail/uncovered_param.rs`。

---

## E0277：trait bound 不满足

**这是最常见的一个码**，本书里出现了好几种形态。

### 形态一：`!Unpin`

```text
error[E0277]: `PhantomPinned` cannot be unpinned
  = note: consider using the `pin!` macro
          consider using `Box::pin` if you need to access the pinned value
          outside of the current scope
```

★ **注意编译器指的是那个字段**（`PhantomPinned`），不是整个类型 ——
它告诉你"谁该负责"。

**修法**：用 `Box::pin`（堆）或 `pin!`（栈上隐藏变量）拿到稳定地址。

**章**：4、19。**反例**：`ch04-pin/fail/pin_requires_unpin.rs`。

### 形态二：`!Send`

```text
error[E0277]: `Rc<u64>` cannot be sent between threads safely
  = help: within `{closure@...}`, the trait `Send` is not implemented for `Rc<u64>`
```

**修法**：换 `Arc`；或者缩小跨线程捕获的范围（第 12 章实测：
只捕获一个 `u64` 字段就能过，即使整个类型 `!Send`）。

**章**：12。**反例**：`ch12-send-sync/fail/not_send.rs`。

### 形态三：`!Sync`

```text
error[E0277]: `Cell<u64>` cannot be shared between threads safely
  = note: if you want to do aliasing and mutation between multiple threads,
          use `std::sync::RwLock` or `std::sync::atomic::AtomicU64` instead
  = note: required for `&Cell<u64>` to implement `Send`
```

★ **最后一行是 `Sync` 的定义**：`&T: Send` 需要 `T: Sync`。

**章**：12。**反例**：`ch12-send-sync/fail/not_sync.rs`。

### 形态四：future 的 `Send`

```text
error: future cannot be sent between threads safely
  = help: within `impl Future<Output = u64>`, the trait `Send` is not implemented
          for `std::sync::MutexGuard<'_, u64>`
note: future is not `Send` as this value is used across an await
  |
4 |     let g = m.lock().unwrap();
  |         - has type `std::sync::MutexGuard<'_, u64>` which is not `Send`
5 |     std::future::ready(()).await;
  |                            ^^^^^ await occurs here, with `g` maybe used later
```

★ **`Send` 由状态机的字段决定**（第 20 章）。
错误信息把三个点都标出来了：哪个值、什么类型、跨过了哪个 `await`。

**修法**：把锁的作用域收进一个块。

⚠️ **`drop(g)` 没有用** —— 编译器数的是**变量**的存活区间：

```text
note: future is not `Send` as this value is used across an await
  |                            ^^^^^ await occurs here, with `g` maybe used later
```

**章**：20。

**反例**：`ch20-async-lifetimes/fail/guard_across_await.rs`、
`fail/drop_does_not_help.rs`、`fail/send_contagion.rs`。

---

## E0283：类型推断歧义

```text
error[E0283]: type annotations needed
```

**含义**：泛型参数版本里，编译器不知道你要哪个 `T`。

**为什么**：`ContainerG<T>` 的 `T` 是**输入**位置上的参数，
调用 `container.get(0)` 时无从推断（第 6 章）。

**修法**：显式标注 `container.get(0) as u64`，
或者改用关联类型（`Self::Item` 由实现唯一确定）。

**章**：6。**反例**：`ch06-associated-types/fail/ambiguous.rs`。

---

## E0308：类型不匹配

```text
error[E0308]: mismatched types
```

★ 本书里最有教学意义的一处是 **variance 方向搞反**（第 3 章）：
把逆变的位置当协变用，编译器报的就是这个平平无奇的 `E0308` ——
但**原因**是子类型关系方向错了。

**章**：3。**反例**：`ch03-variance/fail/contravariance.rs`。

---

## E0382：使用了已移动的值

```text
error[E0382]: borrow of moved value: `s`
```

★ 在 channel 一章里，这个错误是**特性不是 bug**（第 14 章）：

```rust
tx.send(msg).unwrap();
println!("{msg}");      // ← E0382
```

**发送即 move** —— 编译器用这条规则在**编译期**切断了别名。

**章**：14。**反例**：`ch14-channels/fail/use_after_send.rs`。

---

## E0502：借用冲突

```text
error[E0502]: cannot borrow `v` as mutable because it is also borrowed as immutable
   |
22 |     let first = &v[0];
   |                  - immutable borrow occurs here          ← ① 借用的产生点
23 |     v.push(4);
   |                  ^^^^^^^^^ mutable borrow occurs here    ← ② 冲突点
24 |     println!("{first}");
   |                ----- immutable borrow later used here    ← ③ 最后一次使用点
```

★ **这三个点是借用检查器的全部推理过程**（第 1 章）。

**修法**：调**使用点的位置**，而不是调作用域。

**章**：1。**反例**：`ch01-borrow/fail/E0502.rs`、`fail/use_after_conflict.rs`。

---

## E0507：不能移出解引用

```text
error[E0507]: cannot move out of dereference of `Pin<Box<NotUnpin>>`
```

**含义**：想通过解引用拿走所有权。

★ 与 `E0596` 一起，构成 `Pin` 挡住移动的**两条路**（第 19 章）：
拿不到 `&mut`（E0596）、也移不走（E0507）。
**一个值能被移动，只有这两条路。`Pin` 把两条都堵死。**

**章**：19。**反例**：`ch19-pin/fail/move_pinned.rs`。

---

## E0515：不能返回指向局部变量的引用

```text
error[E0515]: cannot return value referencing local variable `local`
```

★ 在生命周期一章里，这个错误常出现在"**过度标注**"的代码上：

```rust
fn tied<'a>(x: &'a str, _y: &'a str) -> &'a str { x }

pub fn use_tied() -> &'static str {
    let local = String::from("temp");
    tied("static str", &local)      // ← E0515
}
```

`'a` 同时约束 `x` 和 `y`，而 `&local` 只能满足很短的寿命 ——
于是 `'a` 被拉短到 `local` 的寿命，返回值就不能是 `'static`。

★ **结论：标注写得越紧，求解空间越小，越容易被拒。**
生命周期标注不是"让编译器更宽容"，是"**你主动承诺的约束**"。

**章**：2。**反例**：`ch02-lifetimes/fail/over_annotated.rs`。

---

## E0596：不能可变借用

```text
error[E0596]: cannot borrow data in dereference of `Pin<Box<NotUnpin>>` as mutable
  = help: trait `DerefMut` is required to modify through a dereference,
          but it is not implemented for `Pin<Box<NotUnpin>>`
```

★ **措辞值得读清楚**：编译器说的是 "cannot borrow ... as mutable"，
**不是**"这个类型不能移动"。

**`Pin` 用"借用的可变性"来表达"能不能移动"** ——
`DerefMut` 那条 impl 对 `!Unpin` **根本不存在**。

**章**：19。**反例**：`ch19-pin/fail/move_pinned.rs`。

---

## E0597：借用的值活得不够长

```text
error[E0597]: `local` does not live long enough
```

**含义**：借用的存活区间超出了被借用值的。

★ 两种高频场景：

1. **不变性**（第 3 章）：`&mut T` 在 `T` 上**不变**，
   于是 `&'static mut T` 不能收缩成 `&'a mut T`；
2. **HRTB 量化方向反了**（第 9 章）：该写 `for<'a> F: Fn(&'a T)` 时
   写成了函数签名上的 `'a`。

**章**：3、9、11。

**反例**：`ch03-variance/fail/invariance.rs`、`ch09-hrtb/fail/too_weak.rs`、
`ch09-hrtb/fail/no_hrtb_for_visitor.rs`、`ch11-project-abstract/fail/too_weak_bound.rs`。

---

## E0733：`async fn` 不能递归

```text
error[E0733]: recursion in an async fn requires boxing
  = note: a recursive `async fn` call must introduce indirection such as
          `Box::pin` to avoid an infinitely sized future
```

★ **编译器直接说出了原因**："to avoid an infinitely sized future"。
**这个错误码本身就证明了"状态机大小"这个概念**（第 18 章）。

**修法**：`Box::pin` 把内层的大小变成一个指针（8 字节）。

**章**：18。**反例**：`ch18-future/fail/recursive_async.rs`。

---

## 非错误码的几条

有几种错误**没有 E 码**，但在本书里很关键：

### `invalid_reference_casting`（lint，`deny` 默认）

```text
error: assigning to `&T` is undefined behavior, consider using an `UnsafeCell`
   |
30 |     let p = x as *const u64 as *mut u64;
   |             --------------------------- casting happened here
32 |     unsafe { *p = v }
   |              ^^^^^^
   = note: `#[deny(invalid_reference_casting)]` on by default
```

★ **不是 Miri 报的，是 rustc 自己报的**，而且**默认 deny**。
它把**铸型点**和**写入点**一起标出来 —— UB 是这两件事**合起来**造成的。

⚠️ 但它是**局部的**：只看得到同一函数体内的铸型 → 写入链。
跨函数 + `black_box` 就抓不到（只有 Miri 能）。

**章**：25。**反例**：`ch25-aliasing/fail/write_through_shared_ref.rs`。

### `async_fn_in_trait`（lint，`warn` 默认）

```text
warning: use of `async fn` in public traits is discouraged as auto trait
         bounds cannot be specified
  = note: you can suppress this lint if you plan to use the trait only in your
          own code, or do not care about auto traits like `Send` on the Future
  = note: `#[warn(async_fn_in_trait)]` on by default
```

★ 这条 lint **就是 AFIT 现状的浓缩**（第 21 章）：
`async fn` 在 trait 里能用了，但**你无法表达"返回的 future 是 `Send`"**
—— 而 `tokio::spawn` 恰恰要求这个。

**章**：21。

### `lifetime may not live long enough`

```text
error: lifetime may not live long enough
```

★ 省略规则"不够用"时的措辞（注意：**不是** E 码）。
常见于 HRTB（第 9 章）和 GAT（第 10 章）：
普通关联类型表达不了"借自 `self` 的迭代器"，需要 `type Item<'a>` +
`where Self: 'a`。

**章**：2、9、10。

---

## 索引速查

| 码 / 名称 | 一句话 | 章 |
|---|---|---|
| **E0038** | trait 建不出 vtable | 7、10、11 |
| **E0106** | 缺少生命周期标注 | 2、20 |
| **E0117** | 孤儿规则：外部 trait + 外部类型 | 8 |
| **E0119** | 实现冲突（含 blanket） | 6、8、11 |
| **E0210** | 类型参数没被本地类型覆盖 | 8 |
| **E0277** | bound 不满足（`!Unpin` / `!Send` / `!Sync`） | 4、12、19、20 |
| **E0283** | 类型推断歧义 | 6 |
| **E0308** | 类型不匹配（variance 方向反了） | 3 |
| **E0382** | 使用已移动的值（发送即 move） | 14 |
| **E0502** | 借用冲突 | 1 |
| **E0507** | 不能移出解引用（`Pin`） | 19 |
| **E0515** | 不能返回局部变量的引用 | 2 |
| **E0596** | 不能可变借用（`Pin`） | 19 |
| **E0597** | 活得不够长（不变性 / HRTB 方向） | 3、9、11 |
| **E0733** | `async fn` 不能递归 | 18 |
| `invalid_reference_casting` | 通过 `&T` 写 | 25 |
| `async_fn_in_trait` | AFIT 的 `Send` 问题 | 21 |

---

## 怎么用这张表

**不要**背错误码。用它的方式是这样的：

1. 你遇到一个报错 → 在表里找到码；
2. 跳到对应章节，看**这一章的 MIR / 汇编证据**；
3. 你会发现问题几乎总在"**你以为的规则**"和"**编译器实际在量的东西**"
   之间 —— 而这本书每一章都在拆这个差。

★ 最值得记住的一条来自第 1 章：

> **编译器报错时给你标出的那几个点，就是它的全部推理过程。**
> 读那些点，而不是读那句话。
