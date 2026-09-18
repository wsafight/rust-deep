# 11. 实战：设计一个小型 trait 抽象层

> 一句话：**第 6–10 章的判据不是并列的清单，是互相牵制的。**
> 你在第 6 章为了表达力选了泛型参数，第 7 章就会发现 `dyn` 没了；
> 你在第 10 章为了 lending 选了 GAT，第 7 章又会发现 `dyn` 没了。
> **每一处"更强的表达力"，都在削弱"动态分发"。**

## 11.0 需求

本章的 API 是一个**事件存储的投影层**：能 append 事件、能按 key 读出投影。

需求会**逐步加码**，每一步都逼出一个判据：

| 步骤 | 需求 | 逼出的判据 | 来自 |
|---|---|---|---|
| 1 | 能求和、能计数 | 一个类型实现几次 | 第 6 章 |
| 2 | 投影器要能放进 `Vec<Box<...>>` | 能不能建 vtable | 第 7 章 |
| 3 | 投影要按 key 分类 | 一族类型 | 第 10 章 |
| 4 | 投影要能借出内部数据 | 生命周期参数的 GAT | 第 10 章 |
| 5 | 流水线要能接受任意生命周期的输入 | 量化方向 | 第 9 章 |
| 6 | 给所有投影器自动加一个能力 | 谁拥有 trait | 第 8 章 |

**每一步都能编译通过，但每一步都在悄悄关掉一扇门。**
本章的价值就在于把那些"悄悄关上的门"指出来。

## 11.1 第 1 步：最小的抽象 —— 关联类型

```rust
pub trait Projection {
    type Out;
    fn project(&self, events: &[u64]) -> Self::Out;
}

pub struct Sum;
impl Projection for Sum { type Out = u64; fn project(&self, e: &[u64]) -> u64 { e.iter().sum() } }

pub struct Count;
impl Projection for Count { type Out = usize; fn project(&self, e: &[u64]) -> usize { e.len() } }
```

**为什么是关联类型而不是泛型参数？** 用第 6 章的判据走一遍：

> 一个 `Sum` 能实现 `Projection` 几次？**一次。**

`Sum` 的投影结果只有一个（`u64`）。两个不同的投影是**两个不同的类型**
（`Sum` 和 `Count`），不是"一个类型实现两次"。

**所以关联类型是正确选择**，泛型参数在这里是多余的。

MIR 里两个 impl 各自单态化（`.evidence/ch11-project-abstract-lib.mir`）：

```mir
fn <impl at examples/ch11-project-abstract/src/lib.rs:45:1: 45:24>::project(_1: &Sum, _2: &[u64]) -> u64
fn <impl at examples/ch11-project-abstract/src/lib.rs:56:1: 56:26>::project(_1: &Count, _2: &[u64]) -> usize
```

```mir
_0 = <Sum as Projection>::project(move _2, copy _1) -> [return: bb1, unwind continue];
_0 = <Count as Projection>::project(move _2, copy _1) -> [return: bb1, unwind continue];
```

**没有间接调用，没有 vtable** —— 静态分发，零成本。

## 11.2 第 2 步：放进 `Vec<Box<dyn ...>>`

需求变了：投影器要能**运行时收集**（比如从配置里读出来），
所以得能装进 `Vec<Box<dyn Projection<Out = u64>>>`。

```rust
pub fn sum_dyn(events: &[u64], p: &dyn Projection<Out = u64>) -> u64 {
    p.project(events)
}
```

**能编译。** `Projection` 是 **dyn compatible**：

- 没有泛型方法；
- 没有返回 `Self`；
- 关联类型 `Out` 只在返回位置出现，而且被写成了 `Out = u64`。

★ **这里有一个第 6 章没讲的隐藏权重**：
**关联类型不破坏 dyn compatibility，泛型方法才破坏。**

### `dyn` 的真实代价：6 条指令

`sum_dyn`（符号导出，编译器不知道调用者是谁）的完整函数体（`-O`）：

```asm
__RNvCsrtIYgyWToU_3lib7sum_dyn:
	mov	x8, x1
	mov	x1, x0
	ldr	x3, [x3, #24]     ; ← vtable 偏移 24（与第 7 章完全一致）
	mov	x0, x2
	mov	x2, x8
	br	x3                ; ← 间接尾跳转
```

**6 条指令。** 其中 `ldr` + `br` 就是动态分发的全部代价 ——
和第 7 章的 `ldr x1, [x1, #24]` + `br x1` 是同一件事。

### ★ 但 `dyn` 的代价**只在类型真的未知时**才付

这是本章最有价值的一条实测结果。同一个 `sum_dyn`，在调用点类型已知时：

```rust
#[unsafe(no_mangle)]
pub fn use_dyn(events: &[u64]) -> u64 { sum_dyn(events, &Sum) }
```

实测 `use_dyn` 的函数体里，`br x?` / `blr` 出现 **0 次**：

```bash
awk '/^_use_dyn:/,/cfi_endproc/' .evidence/ch11-project-abstract-lib.O3.s \
  | grep -cE 'br\s+x[0-9]+|blr'
# → 0
```

LLVM **完全去虚化**了：它知道 `p` 就是 `&Sum`，
于是把 vtable 加载**常量折叠掉**，直接内联了 `Sum::project`。

> **这是对第 7 章那句"`dyn` 的代价是一次 `ldr` + 一次 `br`"的补完：**
> 那两条指令**只在编译器不知道具体类型时才付**。
> 一旦调用点类型可见（哪怕参数写的是 `&dyn`），整个 vtable 加载就消失了。
>
> 所以"我用了 `dyn` 所以慢"是一个**不成立**的推论 ——
> 要看的是**调用点能不能看见具体类型**。

★ 这条断言必须限定在**函数体内**：`.O3.s` 里有几百个符号，
`br x?` 到处都是，全文件 grep 说明不了任何事。
为此给 `verify-all.sh` 加了第四种断言类型
**`assert_fn_contains` / `assert_fn_not_contains`**
（awk 取 `^符号:` 到 `cfi_endproc` 之间的函数体）。

### 第一扇门关上了：泛型方法

需求又变了：用户想"顺便投影成别的类型"。

```rust
pub trait Projection {
    type Out;
    fn project(&self, events: &[u64]) -> Self::Out;
    fn project_as<T>(&self, events: &[u64]) -> T;   // ← 加这个
}
```

```text
error[E0038]: the trait `Projection` is not dyn compatible
   |
29 | pub fn use_dyn(p: &dyn Projection<Out = u64>, e: &[u64]) -> u64 {
   |                    ^^^^^^^^^^^^^^^^^^^^^^^^^ `Projection` is not dyn compatible
   |
26 |     fn project_as<T>(&self, events: &[u64]) -> T;
   |        ^^^^^^^^^^ ...because method `project_as` has generic type parameters
```

**`dyn` 没了。** 理由还是第 7 章那句：vtable 是定长的，
而 `project_as` 的每个 `T` 都需要一个槽位，`T` 有无穷多个。

★ **所以"关联类型还是泛型参数"这个选择，不只看第 6 章那条判据，
还要看"会不会毁掉 `dyn`"。** 这是判据之间的第一次牵制。

## 11.3 第 3 步：按 key 分类 —— GAT

需求：投影要按 key 分组（`HashMap<K, u64>`）。

如果 `K` 只能是 `u64`，关联类型就够了。但 `K` 是**开放的** ——
用户可能用 `u64`、`String`、自定义 ID 类型。

用第 6 章的判据：**`KeyedSum` 能实现这个 trait 几次？**
如果写成 `trait KeyedProjection<K>`，那就是"每个 `K` 一次"——
**而 `K` 是写不完的**。

所以这里要 GAT：

```rust
pub trait KeyedProjection {
    type Out<K>;
    fn project_keyed<K: Clone + Eq + Hash>(&self, events: &[(K, u64)]) -> Self::Out<K>;
}

pub struct KeyedSum;
impl KeyedProjection for KeyedSum {
    type Out<K> = HashMap<K, u64>;
    fn project_keyed<K: ...>(&self, events: &[(K, u64)]) -> HashMap<K, u64> { ... }
}
```

**一个 impl 覆盖了所有 `K`。**

★ 顺带一条容易搞混的规则（本章实测）：

| GAT 参数 | 需要 `where Self: 'a` 吗 |
|---|---|
| 生命周期 `type Out<'a>` | ✅ 强制 |
| 类型 `type Out<K>` | ❌ 不需要 |

`where Self: 'a` 是**生命周期参数**的 GAT 才需要的约束。
参数是类型时没有这个问题（`K` 和 `Self` 的存活时间没关系）。

### 第二扇门关上了：`dyn`

GAT **本身**就足以让 trait 失去 dyn compatibility（第 10 章的 E0038：
`because it contains generic associated type Out`）。

所以 `KeyedProjection` **只能是静态分发的**。
"运行时收集一堆投影器"这个需求，在 GAT 这一步没法满足了。

## 11.4 第 4 步：借出内部数据 —— 生命周期参数的 GAT

需求：投影不想返回 `HashMap`（拷贝），而是想**借出**内部缓存。

```rust
pub trait BorrowingProjection {
    /// ★ 生命周期参数的 GAT：`where Self: 'a` 是**强制**的（第 10 章）。
    type Out<'a> where Self: 'a;
    fn project_borrowed<'a>(&'a self, events: &'a [u64]) -> Self::Out<'a>;
}

pub struct CachedSum { cache: Vec<u64> }

impl BorrowingProjection for CachedSum {
    type Out<'a> = &'a [u64] where Self: 'a;      // ← 借自 self
    fn project_borrowed<'a>(&'a self, _: &'a [u64]) -> &'a [u64] { &self.cache }
}
```

这正是第 10 章 `Chunks` 那个例子的形状：
**返回的引用借自 `self`，而不是借自参数。**

MIR 里 `<CachedSum as BorrowingProjection>::project_borrowed` 的签名
会是 `(&CachedSum, &[u64]) -> &[u64]` —— GAT 参数已经消失。

## 11.5 第 5 步：流水线 —— `for<'a>`

需求：一个流水线，对每个投影器调用一次，并且**在内部造临时数据**：

```rust
pub fn run_pipeline<F>(events: &[u64], f: F) -> u64
where
    F: for<'a> Fn(&'a [u64]) -> u64,        // ← 必须 for<'a>
{
    let doubled: Vec<u64> = events.iter().map(|x| x * 2).collect();
    f(&doubled) + f(events)
}
```

写成 `fn run_pipeline<'a, F: Fn(&'a [u64]) -> u64>(events: &'a [u64], f: F)` 就报（`fail/too_weak_bound.rs`）：

```text
error[E0597]: `doubled` does not live long enough
   |
18 | pub fn run_pipeline<'a, F>(events: &'a [u64], f: F) -> u64
   |                     -- lifetime `'a` defined here
22 |     let doubled: Vec<u64> = events.iter().map(|x| x * 2).collect();
   |         ------- binding `doubled` declared here
23 |     f(&doubled) + f(events)
   |     --^^^^^^^^-
   |     | |
   |     | borrowed value does not live long enough
   |     | argument requires that `doubled` is borrowed for `'a`
```

与第 9 章那条判据完全一致：**"我能不能用自己临时造的数据去调这个闭包？"**
能 → 需要 `for<'a>`。

## 11.6 第 6 步：自动加能力 —— coherence

需求：给**所有**实现了 `Projection` 的类型，自动提供"投影并计数"。

```rust
pub trait Counted {
    fn project_count(&self, events: &[u64]) -> usize;
}

impl<T: Projection> Counted for T {
    fn project_count(&self, events: &[u64]) -> usize {
        let _ = self.project(events);
        events.len()
    }
}
```

**这是合法的 blanket impl**，因为 `Counted` 是**本地的** trait（第 8 章）。
如果 `Counted` 是别人的 trait，这里就是 E0210。

### 第三扇门关上了：特化

但你不能**同时**给所有类型一个默认实现，再给 `Sum` 一个特化版本
（`fail/conflicting_blanket.rs`）：

```text
error[E0119]: conflicting implementations of trait `AllProjections` for type `Sum`
   |
22 | impl<T> AllProjections for T {
   | ---------------------------- first implementation here
   |
28 | impl AllProjections for Sum {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^ conflicting implementation for `Sum`
```

**这就是为什么 stable Rust 里没有特化。**
blanket impl 已经覆盖了那个类型，特化版本就撞上了。

## 11.7 五扇门的全貌

把本章的牵制关系画成一张表：

| 想要的能力 | 代价 | 判据 |
|---|---|---|
| 泛型方法（`project_as<T>`） | ❌ `dyn` 没了（E0038） | 第 7 章 |
| GAT（`Out<K>` / `Out<'a>`） | ❌ `dyn` 没了（E0038） | 第 10 章 |
| blanket impl | ❌ 特化没了（E0119） | 第 8 章 |
| 借用 `self` 的投影 | ❌ 必须 GAT，于是 `dyn` 没了 | 第 10 章 |
| 接受任意生命周期的闭包 | ⚠️ 必须 `for<'a>`（写错就 E0597） | 第 9 章 |
| 关联类型（`Out`） | ✅ **不影响** `dyn` | 第 6 章 |

★ **最后一行是关键**：关联类型是**唯一**"不牺牲 `dyn`"的表达力升级。
这也解释了为什么 `Iterator` 用关联类型（`Item`）而不是泛型参数 ——
因为标准库**必须**保住 `Box<dyn Iterator<Item = u8>>`。

**设计 trait 抽象层时的判据顺序**：

1. 先问"**要不要 `dyn`**"？要 → 关掉泛型方法和 GAT；
2. 再问"**一个类型实现几次**"？一次 → 关联类型；一族 → GAT（并接受失去 `dyn`）；
3. 再问"**要不要 blanket impl**"？要 → 接受没有特化；
4. 最后问"**闭包要接受多长的生命周期**"？任意 → `for<'a>`。

## 11.8 亲手验证

```bash
tools/evidence.sh ch11-project-abstract
scripts/verify-all.sh ch11

# dyn 的真实代价（6 条指令，函数体内）
awk '/^__RNvCsrtIYgyWToU_3lib7sum_dyn:/,/cfi_endproc/' .evidence/ch11-project-abstract-lib.O3.s

# ★ 去虚化：0 次间接跳转
awk '/^_use_dyn:/,/cfi_endproc/' .evidence/ch11-project-abstract-lib.O3.s \
  | grep -cE 'br\s+x[0-9]+|blr'
```

**怎么算验证成功**：

1. `sum_dyn` 的函数体里有 `ldr x3, [x3, #24]` 和 `br x3`（6 条指令）；
2. **同一个 trait**，`use_dyn` 的函数体里 `br`/`blr` 出现 **0 次**
   —— 这是本章最有价值的一条；
3. `fail/generic_method_kills_dyn.rs` 报 **E0038**，note 说
   `because method project_as has generic type parameters`；
4. `fail/conflicting_blanket.rs` 报 **E0119**（特化不可用）；
5. `fail/too_weak_bound.rs` 报 **E0597**。

```bash
scripts/verify-all.sh ch11      # 10 条断言
```

## 11.9 与 unsafe 的关系

这一章没有出现 `unsafe`，但它**决定了你什么时候必须用它**。

上表里每一扇"关上的门"，在工程上只有三种出路：

1. **接受限制**（大多数情况）—— 换一个不需要 `dyn` / 不需要 GAT 的设计；
2. **换一个抽象层**—— 比如把 lending 和 `dyn` 拆成两个 trait
   （标准库就是这么做的：`Iterator` 和 `LendingIterator` 是两个 trait）；
3. **用 `unsafe` 绕过类型系统的限制**—— 比如手工做类型擦除、
   手工维护一张函数指针表来模拟 GAT 的动态分发。

第 3 条是 `unsafe` 在抽象层设计里的典型用途，也是第 24 章的主题。
**但注意：本章的每一个"门"都是 soundness 的边界** ——
绕过去意味着你要自己维持编译器本来会帮你维持的保证。

★ 具体到本章的 API，一个常见的 `unsafe` 出路是
**手工类型擦除**：把 `dyn Projection` 拆成
`(数据指针, 手工函数表)` 两个 `*const ()`，自己管理生命周期。
标准库的 `Any` 就是这么干的。**代价是：所有保证都变成你的责任。**

## 11.10 小结

- **五个判据互相牵制**：表达力 ↔ 动态分发，是一对基本矛盾。
- **关联类型是唯一"不牺牲 `dyn`"的表达力升级** ——
  这就是 `Iterator` 用 `type Item` 而不是泛型参数的原因。
- **泛型方法和 GAT 都会毁掉 `dyn`**（E0038），理由都是第 7 章那句
  "vtable 是定长的"。
- **`dyn` 的代价只在类型真的未知时才付**：实测 `use_dyn`（调用点类型已知）
  的函数体里间接跳转 **0 次**，LLVM 完全去虚化。
- **blanket impl 与特化不可兼得**（E0119）—— 这是 stable 没有特化的原因。
- **`where Self: 'a` 只有生命周期参数的 GAT 才需要**，
  类型参数的 GAT（`Out<K>`）不需要。
- **设计顺序**：先定 `dyn` 要不要 → 再定关联类型还是 GAT →
  再定 blanket impl → 最后定闭包的量化方向。

第二部分到此结束。我们有了完整的 trait 系统工具箱：
关联类型、`dyn`、coherence、HRTB、GAT，以及它们之间的取舍。

下一部分转向**并发**：`Send` / `Sync` 到底是什么，
以及 `Arc::clone` 的原子操作在硬件上长什么样。
