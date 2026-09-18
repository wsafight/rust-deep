# 10. GAT：泛型关联类型

> 一句话：关联类型本来是"**一个**类型"，GAT 把它变成"**一族**类型"——
> `type Item<'a>` 是 `Self` 的函数 `'a -> Type`。
> 它解决的正是 `Iterator` 解决不了的那个问题：**`next` 返回的东西不能借用 `self`。**

## 10.0 一个会让你卡住的例子

你想写一个"按窗口滑动"的迭代器，数据是它**自己拥有**的：

```rust
pub struct Chunks {
    data: Vec<u64>,
    i: usize,
}

pub trait LendingIter {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;
}

impl LendingIter for Chunks {
    type Item = &'static [u64];        // ← 只能这么写？
    fn next(&mut self) -> Option<Self::Item> {
        if self.i + 2 <= self.data.len() {
            let w = &self.data[self.i..self.i + 2];
            self.i += 1;
            Some(w)
        } else { None }
    }
}
```

```text
error: lifetime may not live long enough
  --> src/lib.rs:35:13
   |
31 |     fn next(&mut self) -> Option<Self::Item> {
   |             - let's call the lifetime of this reference `'1`
...
35 |             Some(w)
   |             ^^^^^^^ returning this value requires that `'1` must outlive `'static`
```

`&'static [u64]` 是**唯一能通过类型检查**的写法（因为 `Item` 必须是一个具体类型），
而它立刻被拒绝——`self` 活得没有 `'static` 长。

你会想："那我写 `type Item = &[u64]` 不就行了？" 实测：

```text
error: missing lifetime in associated type
  --> src/lib.rs:5:17
   |
4 | impl<'s> LendingIter for Windows<'s> {
   |     ---- there is a named lifetime specified on the impl block you could use
5 |     type Item = &[u64];
   |                 ^ this lifetime must come from the implemented type
   |
   = note: in the trait the associated type is declared without lifetime parameters,
           so using a borrowed type for them requires that lifetime to come from the
           implemented type
help: consider using the lifetime from the impl block
   |
5 |     type Item = &'s [u64];
   |                  ++
```

编译器的建议是"用 impl 块上的 `'s`"。**但如果数据是 `self` 自己的，就没有 `'s` 可用。**

**"让 `Item` 的生命周期随每次 `next` 调用变化"——普通关联类型表达不了这件事。**
这一章讲 GAT 怎么表达。

## 10.1 表层解释（官方书会怎么讲）

官方书会说：

- **GAT**（generic associated type）就是"带参数（生命周期或类型）的关联类型"：
  `type Item<'a>;`；
- 它主要用于 **lending iterator**（借出内部数据的迭代器）；
- 需要写 `where Self: 'a`。

这些都对，但"关联类型带参数"这句话容易被理解成
"多了一个语法糖"。它不是——**它是表达能力的扩展**：

| | 关联类型 | GAT |
|---|---|---|
| 数学形状 | `Self -> Type` | `Self -> ('a -> Type)` |
| 一个 `Self` 对应 | **一个**类型 | **一族**类型 |

**这一章要说明的是"族"这个概念**——它既是 lending iterator 的解法，
也是"一族类型"的解法。

## 10.2 编译器眼里的样子

### 10.2.1 GAT 的形状

```rust
pub trait LendingIter {
    type Item<'a>
    where
        Self: 'a;

    fn next(&mut self) -> Option<Self::Item<'_>>;
}
```

三处细节都**不能少**：

1. `type Item<'a>` —— 关联类型带一个生命周期参数；
2. `where Self: 'a` —— **强制要求**（见 10.2.3）；
3. `Self::Item<'_>` —— 使用处要**代入**一个具体的生命周期，
   这里用 `'_`（即 `&mut self` 的那个）。

实现：

```rust
impl LendingIter for Chunks {
    type Item<'a> = &'a [u64] where Self: 'a;

    fn next(&mut self) -> Option<&[u64]> {
        if self.i + 2 <= self.data.len() {
            let w = &self.data[self.i..self.i + 2];
            self.i += 1;
            Some(w)
        } else { None }
    }
}
```

`Item<'a>` 的具体类型是 `&'a [u64]`，**借自 `self`**。
每次 `next` 借用 `self` 时，`'a` 取那个借用的生命周期 —— 这就是"族"的含义。

### 10.2.2 MIR 里 GAT 完全消失了

`.evidence/ch10-gat-lib.mir`：

```mir
_4 = <Windows<'_> as LendingIter>::next(move _5) -> [return: bb2, unwind continue];
_4 = <Chunks        as LendingIter>::next(move _5) -> [return: bb2, unwind: bb10];
```

而 `Chunks::next` 的定义（第 4 行）：

```mir
fn <impl at examples/ch10-gat/src/lib.rs:55:1: 55:28>::next(_1: &mut Chunks) -> Option<&[u64]>
```

**签名里完全没有 GAT 的痕迹。** `Item<'a>` 在单态化时就被代入了，
只剩一个普通的 `Option<&[u64]>`。

这和"生命周期被完全擦除"（第 2 章）、"`Pin` 是零成本"（第 4 章）
是同一类现象：**编译期的量词在最终代码里不留痕迹**。

### 10.2.3 `where Self: 'a` 是强制的，不是风格

漏掉它会报（`fail/missing_where.rs`）：

```text
error: missing required bound on `Item`
  --> examples/ch10-gat/fail/missing_where.rs:17:5
   |
17 |     type Item<'a>;
   |     ^^^^^^^^^^^^^-
   |                  |
   |                  help: add the required where clause: `where Self: 'a`
   |
   = note: this bound is currently required to ensure that impls have maximum flexibility
   = note: we are soliciting feedback, see issue #87479 <https://github.com/rust-lang/rust/issues/87479>
```

★ 编译器**直接给出修复建议**，note 里还带着 issue 链接（#87479）——
说明这是"**临时强制、等待未来放宽**"的状态。在 1.98.1 上它仍然强制。

语义上的理由：`Self::Item<'a>` 只有在 `Self: 'a` 时才有定义。
否则"`Self` 里可能含有比 `'a` 更短命的东西"，而 `Item<'a>` 又依赖 `Self`，
循环就说不通了。

### 10.2.4 GAT 的参数不限于生命周期

```rust
pub trait Family {
    type Member<T>;                 // ← 类型参数
    fn wrap<T>(v: T) -> Self::Member<T>;
    fn unwrap<T>(m: Self::Member<T>) -> T;
}

pub struct Wrapper;

impl Family for Wrapper {
    type Member<T> = Vec<T>;
    fn wrap<T>(v: T) -> Vec<T> { vec![v] }
    fn unwrap<T>(m: Vec<T>) -> T { m.into_iter().next().unwrap() }
}
```

MIR 里（第 448 行）：

```mir
_2 = <Wrapper as Family>::wrap::<u64>(copy _1) -> [return: bb1, unwind continue];
```

**`Wrapper: Family` 只有一个 impl**，但它覆盖了**所有** `T`。

### 10.2.5 GAT vs trait 泛型参数

同样表达"一族类型"，另一种写法是把参数放到 trait 上：

```rust
pub trait Family2<T> { type Member; fn wrap(v: T) -> Self::Member; }

impl<T> Family2<T> for Wrapper2 { type Member = Vec<T>; fn wrap(v: T) -> Vec<T> { vec![v] } }
```

两者都能编译，但语义不同：

| | GAT | trait 泛型参数 |
|---|---|---|
| `Wrapper: Family` 有几个 impl | **1 个** | `T` 有多少个就多少 impl |
| 一个 impl 能覆盖几个 `T` | **所有** `T` | 只有那一个 |
| 能否 `dyn` | ❌ E0038 | ✅（如果没有 GAT 的话） |

**"所有 `T`"是写不完的。** 这就是 GAT 在类型参数场景下的价值。

### 10.2.6 ★ 零成本：GAT 版本 ≈ 手写循环

用 GAT 迭代器写 `sum_windows`，再手写一个 while 循环 `sum_windows_manual`，
两者在 `-O` 下对比：

| | 行数 | `add.2d` 条数 |
|---|---|---|
| `sum_windows`（GAT） | **61** | **11** |
| `sum_windows_manual`（手写） | **62** | **11** |

```asm
; GAT 版本
_sum_windows:
	cmp	x1, #2
	b.hs	LBB3_2
	...
	add.2d	v0, v4, v0      ; ← 128-bit 向量化 + 4 路展开

; 手写版本
_sum_windows_manual:
	cmp	x1, #2
	b.hs	LBB4_2
	ldr	x11, [x0]       ; ← 唯一的差异：把 v[0] 提到循环外
	...
	add.2d	v5, v5, v0
```

`diff` 之后只差一处：手写版本多一条 `ldr x11, [x0]`，
因为 LLVM 对两个版本选了**略微不同的向量化策略**
（一个用 `dup.2d v0, x11` 广播首元素，一个用重叠加载）。

**GAT 不引入任何额外开销。** 61 vs 62 行、都是 11 条 SIMD 加法。

> ⚠️ 这里只说"指令数几乎相同"，**不说"哪个更快"** ——
> 本书在没有 benchmark 数据之前不写性能断言（见 PLAN §13 断点 6）。

## 10.3 为什么必须这样设计

### 为什么普通关联类型做不到

因为它的数学形状是 `Self -> Type`：**一个输入，一个输出**。
`Chunks::Item` 必须是一个**固定的**类型，而"借自 `self` 的窗口"
在每次调用时的生命周期都不同 —— 它不是一个固定类型。

加一个生命周期参数，形状变成 `Self -> ('a -> Type)`：
对**每个** `'a` 给出一个类型。这才是"每次调用都不同"的正确表达。

> 这与第 6 章的判据是同一个思路：
> **"一个 `Self` 对应几个类型？"**
> 一个 → 关联类型；一族 → GAT（或 trait 泛型参数）。

### 为什么 `where Self: 'a` 必须强制

`Item<'a>` 是 `Self` 的函数。如果 `Self` 里含有比 `'a` 更短命的东西，
那么"用 `'a` 去索引 `Self`"就没有意义 —— `Self` 自己都活不到 `'a`。

`Self: 'a` 这个约束把这种情形排除掉了：
**只有当 `Self` 活得比 `'a` 长时，`Item<'a>` 才有定义。**

编译器 note 里那句 "to ensure that impls have maximum flexibility"
说明这是一个**保守的强制**——未来可能放宽（issue #87479），
但当前版本必须写。

### 为什么 GAT 和 `dyn` 不能共存

第 7 章讲过：**vtable 是一张定长的表**。
`type Item<'a>` 的参数 `'a` 可以有无穷多个取值，
"每个 `'a` 一个槽位"根本写不下。

实测（`fail/gat_not_dyn.rs`）：

```text
error[E0038]: the trait `LendingIter` is not dyn compatible
   |
22 | pub fn use_dyn(x: &mut dyn LendingIter) {
   |                            ^^^^^^^^^^^ `LendingIter` is not dyn compatible
   |
note: for a trait to be dyn compatible it needs to allow building a vtable
   |
16 |     type Item<'a>
   |          ^^^^ ...because it contains generic associated type `Item`
   = help: consider moving `Item` to another trait
```

**和第 7 章的 E0038 是同一个理由**（"不能建 vtable"），
只是这次挡路的是 GAT 而不是泛型方法。

> 所以"用 GAT 表达 lending iterator"和"用 `dyn` 做动态分发"**不能同时要**。
> 这是 GAT 最主要的实际限制——也是为什么 `Iterator` 至今没改成 lending 的。

### 为什么 `Iterator` 不用 GAT

因为 `Iterator` 是**最需要 `dyn` 的 trait 之一**
（`Box<dyn Iterator<Item = u8>>` 到处都是）。
改成 lending 之后，`dyn Iterator` 立刻不可用。

标准库的取舍是：**保留 `dyn`，把 lending 场景交给第三方 trait**
（`lending-iterator`、`async` 的 `Stream` 等）。

## 10.4 反直觉的点

### 反直觉之一：GAT 不是"关联类型加个参数"，是"从值到函数"

`type Item<'a>` 读起来像"多了一个参数"，但它改变的是**种类**（kind）：

- `type Item;` —— `Item` 是一个**类型**（kind: `Type`）；
- `type Item<'a>;` —— `Item` 是一个**类型构造器**（kind: `'a -> Type`）。

**这就是为什么它和 trait 泛型参数不能互相替代**：
泛型参数改变的是"这个 impl 属于谁"，GAT 改变的是"关联类型本身是什么"。

### 反直觉之二：写 `where Self: 'a` 不是"多加保险"，而是唯一自洽的写法

很多人以为 `where Self: 'a` 是编译器"过度保守"。
不是。没有它，`Item<'a>` 的定义域就说不清楚——
**`Self` 活得没 `'a` 长的时候，`Item<'a>` 是什么？**

这个问题没有答案，所以只能把这种情况排除掉。

### 反直觉之三：GAT 和 `dyn` 是**互斥**的，而 `dyn` 常常更重要

这是 GAT 最容易被低估的代价。看 `Iterator` 的例子：

```rust
// 用 GAT 写 lending iterator —— 表达力更强
pub trait LendingIter { type Item<'a> where Self: 'a; fn next(&mut self) -> Option<Self::Item<'_>>; }

// 但它做不了这件事（E0038）
fn sum_all(x: &mut dyn LendingIter) -> u64 { ... }
```

**表达力增强的代价，往往是动态分发能力的丧失。**
判据还是那句：**vtable 装得下吗？**

### 反直觉之四：GAT 完全零成本

`type Item<'a> = &'a [u64]` 看起来像"运行期会有一张类型表"。
实测：**GAT 版本和手写循环的汇编几乎逐行相同**（61 vs 62 行，
都是 11 条 `add.2d`）。

GAT 的参数在**单态化时**就被代入了，运行期没有任何"类型信息"残留。
**"泛型/关联类型/生命周期"在 Rust 里从来不是运行期的东西。**

## 10.5 亲手验证

```bash
tools/evidence.sh ch10-gat
scripts/verify-all.sh ch10

# GAT 的实例化
grep -nE 'LendingIter|Family' .evidence/ch10-gat-lib.mir

# 零成本对照（只差一条 ldr）
diff <(awk '/^_sum_windows:/,/cfi_endproc/'        .evidence/ch10-gat-lib.O3.s) \
     <(awk '/^_sum_windows_manual:/,/cfi_endproc/' .evidence/ch10-gat-lib.O3.s)
```

**怎么算验证成功**：

1. `src/lib.rs` 里的 `Chunks` **编译通过** —— 而 `fail/no_gat.rs` 里
   同样结构的普通关联类型版本**编译失败**（`lifetime may not live long enough`）；
2. MIR 里 `Chunks::next` 的签名是 `Option<&[u64]>`，
   **GAT 参数已经完全消失**；
3. `fail/missing_where.rs` 报 `missing required bound on Item`，
   且 note 里带 issue #87479；
4. `fail/gat_not_dyn.rs` 报 **E0038**，note 说
   `because it contains generic associated type Item`；
5. `sum_windows` 与 `sum_windows_manual` 的汇编行数相差 1 行以内，
   `add.2d` 条数相同。

```bash
scripts/verify-all.sh ch10      # 9 条断言
```

## 10.6 与 unsafe 的关系

这一章和 `unsafe` 的关系很**直接**：**GAT 是"安全的 lending iterator"的前提。**

没有 GAT 时，想实现"借出内部数据"只能靠 `unsafe` 的手工指针操作，
或者用 `Rc` / `RefCell` 把借用检查推到运行期。
GAT 让编译器**在编译期**验证"每次借出的引用不会和下一次借用重叠"。

★ 更重要的是：**GAT 的出现改变了 `unsafe` 抽象的写法**。
比如一个自引用的容器（第 4 章的 `Pin` 场景），
可以用 GAT 表达"取出的视图借自容器本身"：

```rust
pub trait SelfRef {
    type View<'a> where Self: 'a;
    fn view(&self) -> Self::View<'_>;
}
```

**这条约束由编译器检查，而不是由你的 safety comment 检查** ——
这正是"用类型系统消灭 `unsafe`"的典型手法。

反过来，GAT 也是**一个 soundness 陷阱**：如果你在 `unsafe` 实现里
用 `transmute` 伪造 `Item<'a>` 的生命周期（比如把 `&'a T` 变成 `&'static T`），
编译器**不会**发现——因为 GAT 的约束只覆盖类型层，`transmute` 是绕过类型层的。

## 10.7 小结

- **GAT 把关联类型从"一个类型"变成"一族类型"**：
  `type Item;` 是 `Self -> Type`，`type Item<'a>;` 是 `Self -> ('a -> Type)`。
- **它解决的核心问题**：`next` 返回的东西要能借用 `self`
  —— `Iterator` 做不到，lending iterator 需要。
- **`where Self: 'a` 是强制要求**（issue #87479 是"未来可能放宽"的入口），
  漏掉会报 `missing required bound on Item`，且编译器会直接给修复建议。
- **GAT 的参数不限于生命周期**：`type Member<T>` 表达"一族类型"，
  **一个 impl 覆盖所有 `T`**（trait 泛型参数做不到这一点）。
- **GAT 和 `dyn` 互斥**（E0038）：vtable 是定长的，而 `'a` 有无穷多个取值。
  这是 GAT 最主要的实际限制，也是 `Iterator` 至今不用 GAT 的原因。
- **GAT 零成本**：MIR 里 GAT 参数完全消失，
  汇编与手写循环几乎逐行相同（61 vs 62 行，11 条 `add.2d`）。
- **GAT 是消灭 `unsafe` 的工具**：它把"借出的视图不会与下一次借用重叠"
  这条约束交给编译器检查，而不是交给 safety comment。

下一章是第二部分的收尾：**设计一个小型 trait 抽象层**——
把第 6 到 10 章的判据（关联类型 / `dyn` / coherence / HRTB / GAT）
用在同一个真实问题上，看它们怎么互相牵制。
