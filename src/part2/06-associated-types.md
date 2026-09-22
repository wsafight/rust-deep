# 6. 关联类型 vs 泛型参数

> 一句话：判据只有一条——**一个类型能实现这个 trait 几次？**
> 一次 → 关联类型；多次 → 泛型参数。
> 这条判据同时决定了"表达力"和"调用点要不要写类型标注"。

## 先把语法认清

关联类型在 trait 内声明：`trait Iterator { type Item; }`，实现者为每个
`Self` 选择一个确定的 `Item`。trait 泛型参数写成 `trait Convert<T>`，同一
类型可以针对不同 `T` 提供多份实现。前者表达“一对一”，后者表达“一对多”。

可以把关联类型看成**套餐里的固定主食**，把泛型参数看成**点单时再选配菜**。
真正的问题不是哪种写法短，而是选择权究竟属于实现者还是调用者。

### 放到业务里：存储驱动与序列化器

数据库驱动的连接类型通常是实现唯一决定的，适合 `type Connection`；
同一个值要转换成 JSON、字节流或领域 DTO，则可能适合 `Convert<T>`。
选错方向会在 API 扩展时暴露：要么调用点到处需要类型标注，要么你发现同一
实现者根本不能再提供第二种输出。

```rust
trait Driver {
    type Connection;
    fn connect(&self) -> Self::Connection;
}
```

调用方只选 `Driver`，连接类型随实现一起确定；如果选择权应留给调用方，
再把目标类型改成 trait 泛型参数。

## 6.0 一个会让你卡住的例子

你想定义一个"能取出元素"的 trait：

```rust
// 版本 A：关联类型
pub trait Container {
    type Item;
    fn get(&self, i: usize) -> Option<&Self::Item>;
}

// 版本 B：泛型参数
pub trait ContainerG<T> {
    fn get(&self, i: usize) -> Option<&T>;
}
```

两个都能用。什么时候选哪个？

你可能会说"看哪个更方便"。但有一个**硬性区别**：

```rust
impl Container for Numbers { type Item = u64; ... }
impl Container for Numbers { type Item = String; ... }   // ❌ E0119
```

**关联类型版本不允许第二个实现。** 而泛型参数版本允许任意多个：

```rust
impl ContainerG<u64>    for Numbers { ... }   // ✅
impl ContainerG<String> for Numbers { ... }   // ✅
```

这个区别不是"风格"，是**语义**：`Self::Item` 是 `Self` 的**函数**——
一个输入只能有一个输出。而 `ContainerG<T>` 是"`Numbers` 对**每个** `T`
各有一个实现"。

## 6.1 先把常见说法摆上桌

通常会这样概括：

- 关联类型：每个实现只有一个具体类型，用在"trait 只有一个自然的实现"时；
- 泛型参数：同一个类型可以多次实现，用在需要"多态"时；
- 典型例子：`Iterator::Item` 用关联类型，`From<T>` 用泛型参数。

这些都对。但"只有一个自然的实现"是个模糊的说法。
本章把它换成一个**可判定的判据**。

## 6.2 编译器眼里的样子

### 6.2.1 关联类型：第二次实现报 E0119

```text
error[E0119]: conflicting implementations of trait `Container` for type `Numbers`
  |
3 | impl Container for Numbers { type Item = u64; ... }
  | -------------------- first implementation here
4 | impl Container for Numbers { type Item = String; ... }
  | ^^^^^^^^^^^^^^^^^^^^ conflicting implementation for `Numbers`
```

★ 错误是 **E0119（conflicting implementations）**，不是"重复定义"。
编译器说的是"这两个实现**冲突**"——因为它们在回答同一个问题
（`Numbers::Item` 是什么），给出了两个不同答案。

### 6.2.2 泛型参数：三个 impl 都过

```rust
impl Conv<u64>    for Wrapper { ... }
impl Conv<f64>    for Wrapper { ... }
impl Conv<String> for Wrapper { ... }
```

三个 impl 回答的是**不同的问题**（`Wrapper: Conv<u64>`、
`Wrapper: Conv<f64>`、`Wrapper: Conv<String>`），所以不冲突。

### 6.2.3 代价：泛型参数带来推断歧义

```rust
let w = Wrapper(5);
let _y = w.conv();      // 要 u64 还是 f64？
```

```text
error[E0283]: type annotations needed
  |
4 |     let _y = w.conv();
  |                 ^^     ---- type must be known at this point
  |
note: multiple `impl`s satisfying `Wrapper: Conv<_>` found
  |
2 | impl Conv<u64> for Wrapper { fn conv(self) -> u64 { self.0 as u64 } }
3 | impl Conv<f64> for Wrapper { fn conv(self) -> f64 { self.0 as f64 } }
help: consider giving `_y` an explicit type
```

**E0283 是"泛型参数的税"。** 只要上下文能确定类型，就不用标注：

```rust
pub fn conv_with_annotation(w: Wrapper) -> u64 { w.conv() }   // ✅
```

但**关联类型完全不用交这个税**：

```rust
pub fn use_container(c: &Numbers) -> u64 { *c.get(0).unwrap_or(&0) }   // ✅ 无需任何标注
```

因为 `Numbers::Item` 是**唯一确定**的，不存在"用哪个 impl"的问题。

### 6.2.4 生成的代码：两者都是零成本

```asm
_conv_with_annotation:      ; 泛型参数（单态化）
	mov	w0, w0
	ret

_use_container:             ; 关联类型
	ldp	x9, x8, [x0, #8]
	...                     ; 边界检查 + 取值
```

**两者都被完全内联，没有间接调用。**

对比第 7 章的 `dyn`：那里是 `ldr x1, [x1, #24]` + `br x1`。
**关联类型 vs 泛型参数的差别不在运行时，在类型层。**

> 这一点很重要：**两者都不是 `dyn`，都是静态分发。**
> 关联类型不引入动态分发——它只是"这个 trait 的参数由 `Self` 决定"。

## 6.3 为什么必须这样设计

### 关联类型为什么必须唯一？

因为 `Self::Item` 会出现在**很多地方**，而不只是 trait 定义里：

```rust
fn process<C: Container>(c: &C) -> Option<&C::Item> { c.get(0) }
```

`C::Item` 是一个**类型**。如果它可以有多个值，
那 `C::Item` 就不是一个类型了——编译器没法为它生成代码。

**关联类型是"由 `Self` 决定的类型函数"**：
`Item: impl Container -> Type`。
函数当然只能有一个输出。

### 泛型参数为什么允许多次实现？

因为 `trait Conv<T>` 是"一个**二元**关系"：
`(Wrapper, u64)` 和 `(Wrapper, f64)` 是两个不同的命题。
它们可以同时为真。

`From<T>` 就是这么用的：`i32: From<u8>`、`i32: From<u16>`、
`i32: From<i8>`…… 都是独立的事实。

### 为什么 `Iterator::Item` 是关联类型？

因为"一个 `Vec<i32>` 迭代出什么"只有一个答案：`i32`。
如果有多个答案，`for x in v` 就不知道 `x` 是什么类型了。

```rust
pub trait Iterator {
    type Item;                                  // ← 唯一
    fn next(&mut self) -> Option<Self::Item>;
}
```

而 `IntoIterator` **同时**用了两种：

```rust
pub trait IntoIterator {
    type Item;                                  // ← 关联类型：迭代出什么，唯一
    type IntoIter: Iterator<Item = Self::Item>; // ← 关联类型：变成哪个迭代器，唯一
    fn into_iter(self) -> Self::IntoIter;
}

impl<T> IntoIterator for Vec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;      // ← 唯一：就是这一个
}
```

**`Vec<T>` 只有一种"变成迭代器"的方式**——所以用关联类型。

## 6.4 反直觉的点

### 反直觉之一：关联类型不是"更弱的泛型参数"

有人觉得"关联类型只是泛型参数的特例，泛型参数更通用"。
不对——**关联类型能做泛型参数做不到的事**：

```rust
// 这个约束用泛型参数写不出来（或者说要写得很别扭）
fn sum<I>(iter: I) -> I::Item
where
    I: Iterator,
    I::Item: std::ops::Add<Output = I::Item> + Default,
{
    iter.fold(I::Item::default(), |a, b| a + b)
}
```

`I::Item` 出现在**两个位置**（返回类型、`Add` 的约束）。
如果用泛型参数 `I: Iterator<T>`，你就得写：

```rust
fn sum<T, I: Iterator<T>>(iter: I) -> T
where T: Add<Output = T> + Default
```

**这个版本要求调用者显式指定 `T`**，而关联类型版本可以自动推断。
更本质的是：泛型参数版本允许 `I` 对多个 `T` 实现 `Iterator`——
但那不是 `Iterator` 的语义。

### 反直觉之二：`dyn` 只对关联类型"有要求"，对泛型参数没有

```rust
trait A { type Item; fn get(&self) -> Self::Item; }
trait B<T> { fn get(&self) -> T; }

// 两者都能做 trait object 吗？
let _: &dyn A<Item = u64> = ...;   // ✅ 需要指定 Item
let _: &dyn B<u64> = ...;          // ✅ 需要指定 T
```

**两者都需要"把未知填上"**，只是语法不同。
但有个关键区别：**泛型参数让 trait 变成 object-unsafe 的情况更多**——
`trait B<T>` 里的 `T` 如果是方法级别的泛型（不是 trait 级别的），
`dyn B` 就没法表达了。第 7 章会展开。

### 反直觉之三：`impl Trait` 返回位置用的是关联类型的思想

```rust
fn make() -> impl Iterator<Item = u64> { 0..10 }
```

这里的 `Item = u64` 就是关联类型的**约束写法**。
`impl Trait` 的本质是"某个具体的、你不知道名字的类型"——
而它必须**唯一确定**，不能有多个可能。

**这也是为什么 `impl Trait` 不能返回多个类型**：
它和关联类型一样，是"一个函数只能有一个输出"。

## 6.5 亲手验证

```bash
tools/evidence.sh ch06-associated-types
scripts/verify-all.sh ch06

# 反例
rustc --edition 2024 --crate-type=lib examples/ch06-associated-types/fail/conflicting_impl.rs
#   → error[E0119]: conflicting implementations

rustc --edition 2024 --crate-type=lib examples/ch06-associated-types/fail/ambiguous.rs
#   → error[E0283]: type annotations needed
```

**怎么算验证成功**：

1. `fail/conflicting_impl.rs` 报 **E0119** —— 关联类型只能实现一次；
2. `fail/ambiguous.rs` 报 **E0283** —— 泛型参数带来推断歧义；
3. `conv_with_annotation` 和 `use_container` 的汇编里**都没有间接调用**
   —— 两者都是静态分发。

```bash
scripts/verify-all.sh ch06      # 7 条断言
```

## 6.6 与 unsafe 的关系

这一章跟 `unsafe` 关系不大——它是**类型层**的设计选择，
不影响任何运行时保证。

但有一个间接联系：关联类型让 `Iterator` 的元素类型由实现唯一确定，
调用方不必在每一层额外携带和推断一个类型参数。编译器仍可对关联类型投影
做归一化，然后单态化、内联整个迭代器链。

这主要是 API 表达力和类型推断上的收益，不是优化器独有的能力：
泛型参数版本在具体类型已知时同样可以单态化，也可以做到零运行时开销。

## 6.7 小结

- **判据只有一条**：一个类型能实现这个 trait 几次？
  一次 → 关联类型；多次 → 泛型参数。
- **关联类型的语义**：`Self::Item` 是 `Self` 的**函数**，一个输入一个输出。
  第二次实现报 **E0119**。
- **泛型参数的代价**：调用点常常需要类型标注（**E0283**）。
  上下文能确定时不用标注。
- **两者都不引入动态分发**：都完全单态化/内联，生成的代码没有间接调用。
  关联类型 ≠ `dyn`。
- **关联类型不是"更弱的泛型参数"**：`I::Item` 能出现在多个位置并自动推断，
  泛型参数版本往往要求调用者显式指定。
- **`Iterator` 用关联类型让元素类型由实现唯一确定**，改善组合与推断；
  关联类型和泛型参数在具体化后都可以被单态化和内联。

下一章我们把这两条路和第三条路（`dyn`）放在一起：
**vtable 的物理布局决定了动态分发的真实代价**。
