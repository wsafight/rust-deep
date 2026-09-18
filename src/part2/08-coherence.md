# 8. coherence、孤儿规则与 blanket impl

> 一句话：孤儿规则不是"设计者的洁癖"，而是**"每个 (类型, trait) 对至多一个 impl"
> 这条不变量的充分条件**——没有它，`v.to_string()` 到底该调哪个 impl
> 就成了一个不可判定的问题。

## 8.0 一个会让你卡住的例子

你在自己的库里定义了一个 trait，想让它对"所有能打印的东西"都可用：

```rust
// 你的 crate：downstream
use upstream::Format;                 // trait 来自另一个 crate

impl<T: std::fmt::Display> Format for T {
    fn fmt_it(&self) -> String { format!("{self}") }
}
```

```text
error[E0210]: type parameter `T` must be used as an argument to some local type (e.g., `MyStruct<T>`)
 --> src/lib.rs:3:6
  |
3 | impl<T: std::fmt::Display> Format for T {
  |      ^ uncovered type parameter
  |
  = note: implementing a foreign trait is only possible if at least one of the
          types for which it is implemented is local
```

这个 blanket impl 看起来完全无害：它只依赖 `Display`，而 `Display` 是公开的。
**为什么不行？**

更让人费解的是，把 trait 换成你自己的，**一模一样的写法就通过了**：

```rust
pub trait MyDisplay { fn my_fmt(&self) -> String; }
pub trait MyDebug   { fn my_debug(&self) -> String; }

impl<T: MyDisplay> MyDebug for T {          // ✅ 编译通过
    fn my_debug(&self) -> String { format!("MyDebug({})", self.my_fmt()) }
}
```

**同一个 `impl<T> ... for T`，只因为 trait 归属不同，一个行一个不行。**
这一章要说明：判据是"**谁拥有 trait**"，而不是"这个 impl 看起来多合理"。

## 8.1 表层解释（官方书会怎么讲）

官方书会说：

- **孤儿规则**（orphan rule）：`impl 外部trait for 外部类型` 不行，
  trait 和类型至少有一个要是本地的；
- **coherence**：不能有两个 impl 覆盖同一个 `(类型, trait)` 对；
- **blanket impl**：给所有满足某约束的类型实现一个 trait，
  常见于标准库（`impl<T: Display> ToString for T`）；
- 绕过的办法是 **newtype 模式**。

这些都对，但"至少一个是本地的"这句话**不够精确**——
它没法解释 8.0 里那个"trait 归属不同结果就不同"的例子，
也没法解释下面这些实测出来的边界：

| 写法 | 结果 |
|---|---|
| `impl MyDisplay for Vec<u64>` | ✅（trait 是本地的） |
| `impl Display for Wrapped` | ✅（类型是本地的） |
| `impl Display for Box<Local>` | ✅ ← 为什么 `Box` 特殊？ |
| `impl Display for Vec<Local>` | ❌ **E0117** ← 明明也"有本地类型" |
| `impl<T> From<T> for Local<T>` | ✅ |
| `impl<T> From<T> for Vec<Local>` | ❌ **E0210** ← 本地类型出现了，但不够 |

最后两行是关键：**本地类型出现了，也还是不行**。规则比"至少一个是本地的"更细。

## 8.2 编译器眼里的样子

### 8.2.1 官方规则的原文（精确版本）

Rust Reference 的 *Implementations → Orphan rules* 一节，原文：

> Given `impl<P1..=Pn> Trait<T1..=Tn> for T0`, an impl is valid only if at least
> one of the following is true:
>
> - `Trait` is a local trait
> - All of
>   - At least one of the types `T0..=Tn` must be a **local type**.
>     Let `Ti` be the **first** such type.
>   - No **uncovered** type parameters `P1..=Pn` may appear in `T0..Ti` (excluding `Ti`)

把它拆开，就是三件事：

1. **trait 是本地的** → 放行（这就是 8.0 里 `MyDebug` 那半边的答案）；
2. 否则，在 `T0..Tn` 里找**第一个**本地类型 `Ti`；
3. `T0..Ti` 里不能出现"未被覆盖"的类型参数。

"第一个"和"未覆盖"这两个词，就是上面表格里最后两行的全部答案。

### 8.2.2 覆盖规则：本地类型必须出现在**第一个**位置

```rust
pub struct Local(pub u64);

impl<T> From<T> for Local<T>   { ... }   // ✅
impl<T> From<T> for Vec<Local> { ... }   // ❌ E0210
```

第二行报的错（实测）：

```text
error[E0210]: type parameter `T` must be used as an argument to some local type (e.g., `MyStruct<T>`)
  --> examples/ch08-coherence/fail/uncovered_param.rs:17:6
   |
17 | impl<T> From<T> for Vec<Local> {
   |      ^ uncovered type parameter
   |
   = note: implementing a foreign trait is only possible if at least one of the types for which it is implemented is local
   = note: only traits defined in the current crate can be implemented for a type parameter
```

用规则逐步走一遍 `impl<T> From<T> for Vec<Local>`：

- trait 是 `From`，**外部的** → 第 1 条不成立，往下走；
- `T0..Tn` = `[Vec<Local>, T]`。**第一个**本地类型是谁？
  `Vec<Local>` —— `Vec` 是外部的，而且它不是 `#[fundamental]`，
  所以 `Vec<Local>` **不算**本地类型（见 8.2.3）；
  继续往后，`T` 是类型参数、不是本地类型；
- 于是**根本找不到本地类型** → 直接 E0210。

再看 `impl<T> From<T> for Local<T>`：

- `T0 = Local<T>`，`Local` 是本地的 → `Ti = Local<T>`；
- 检查 `T0..Ti`（不含 `Ti`）—— 这个范围是空的；
- 通过。**注意 `T` 出现在 `Ti` 里（`Local<T>`），而 `Ti` 本身不检查**
  —— 这就是"覆盖"（covered）的确切含义。

> **"覆盖"不是"出现过"，而是"出现在第一个本地类型里"。**
> `Vec<Local>` 里 `Local` 出现了，但它被 `Vec` 挡在后面，`T` 并没有被覆盖。

### 8.2.3 fundamental：`Box` 与 `&` 为什么特殊

Reference 紧接着的一句话是理解上面那段的钥匙（原文）：

> Note that for the purposes of coherence, **fundamental types are special**.
> The `T` in `Box<T>` is **not considered covered**, and `Box<LocalType>`
> is considered local.

也就是说：**`Box<T>` 在孤儿规则眼里是"透明的"**——
它不算"覆盖"了 `T`，但它自己**算本地类型**（只要 `T` 是本地类型）。

实测对照（这一对是本章最直观的证据）：

```rust
pub struct Local(pub u64);

impl std::fmt::Display for Box<Local> { ... }   // ✅ 编译通过
impl std::fmt::Display for Vec<Local> { ... }   // ❌ E0117
```

```text
error[E0117]: only traits defined in the current crate can be implemented for types defined outside of the crate
  --> examples/ch08-coherence/fail/orphan_vec.rs:20:1
   |
20 | impl std::fmt::Display for Vec<Local> {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^----------
   |                            |
   |                            `Vec` is not defined in the current crate
```

`&T` / `&mut T` 同样是 fundamental——实测 `impl Display for &Local` 也编译通过。

`#[fundamental]` 在标准库里长这样（`rust-src`，`alloc/src/boxed.rs`）：

```rust
#[lang = "owned_box"]
#[fundamental]
pub struct Box<T: ?Sized, A: Allocator = Global>(Unique<T>, A);
```

**为什么 `Box` 要特殊？** 因为 `Box` 是一个"透明的所有权容器"：
它没有自己的语义，只是把值放到堆上。给它开个口子不会破坏 coherence——
`Box` 不可能给 `Local` 引入新的 impl，也不可能和别人的 impl 撞车。

`Vec` 则不同：它是上游的、可能继续演进的类型。
如果 `Vec<Local>` 算本地类型，那么上游明天给 `Vec<T>` 加一个
`impl<T> From<T> for Vec<T>`，你的代码就会**突然冲突**。

> ★ 附带的一个小发现：`#[fundamental]` 不只用在 `Box` 上。
> `core/src/marker.rs` 里 `Sized` / `MetaSized` / `PointeeSized` 也带着它，
> 源码注释写着 *"for `Default`, for example, which requires that
> `[T]: !Default` be evaluatable"* —— 连 auto trait 的推断也要用到这个概念。

### 8.2.4 E0119 的两种形态（一个可判别的差异）

coherence 的另一半是"两个 impl 不能重叠"。但报错其实有**两种形态**，
而它们的差别恰好说明了冲突的规模。

**形态一：冲突发生在具体类型上**（`fail/conflicting_blanket.rs`）：

```rust
impl<T> MyTrait for T       { fn f(&self) -> u64 { 0 } }   // blanket
impl     MyTrait for u64    { fn f(&self) -> u64 { 1 } }   // 具体类型
```

```text
error[E0119]: conflicting implementations of trait `MyTrait` for type `u64`
   |
14 | impl<T> MyTrait for T { fn f(&self) -> u64 { 0 } }
   | --------------------- first implementation here
16 | impl MyTrait for u64 { fn f(&self) -> u64 { 1 } }
   | ^^^^^^^^^^^^^^^^^^^^ conflicting implementation for `u64`
```

**注意第一行的 `for type \`u64\``** —— 编译器能指出冲突在哪个类型上。

**形态二：冲突发生在泛型层面**（`fail/overlapping_blanket.rs`）：

```rust
impl<T: Copy>  P for T { fn p(&self) -> u64 { 1 } }
impl<T: Clone> P for T { fn p(&self) -> u64 { 2 } }
```

```text
error[E0119]: conflicting implementations of trait `P`
   |
 3 | impl<T: Copy> P for T { fn p(&self) -> u64 { 1 } }
   | --------------------- first implementation here
 4 | impl<T: Clone> P for T { fn p(&self) -> u64 { 2 } }
   | ^^^^^^^^^^^^^^^^^^^^^^ conflicting implementation
```

**这次没有 `for type X`** —— 因为冲突的类型集合是**无限的**，
编译器说不出"对哪个类型冲突"。

> 这一条已经做成了断言：`assert_fails ... 'E0119\]: conflicting implementations of trait .P.$'`
> ——**断言"这行后面没有 `for type`"**，反过来证明这是泛型层面的重叠。

### 8.2.5 blanket impl 编译成了什么

8.0 里那个合法的 blanket impl，在 MIR 里的调用点是：

```mir
fn use_blanket() -> String {
    bb2: {
        _0 = <Vec<u64> as MyDebug>::my_debug(move _1) -> [return: bb3, unwind: bb5];
    }
}
```

写的是 `impl<T: MyDisplay> MyDebug for T`，`Self` 是 `T`；
但调用点已经被**单态化**成 `<Vec<u64> as MyDebug>`。

再看 `-O` 的产物：

```bash
grep -c 'my_debug' .evidence/ch08-coherence-lib.O3.s    # → 0
grep -c 'my_debug' .evidence/ch08-coherence-lib.O3.ll   # → 0
```

**一个独立符号都没生成**，`my_debug` 被完全内联进了 `use_blanket`。

★ 有个容易踩的坑：`.O3.ll` 里其实**还能搜到** `my_debug`，但它在
`!dbg` 元数据里，不在 `define` 里：

```llvm
!65 = distinct !{!65, !"_RNvXs1_...VecyENtB5_7MyDebug8my_debugB5_"}
```

调试信息保留了原始符号名，但那**不是函数定义**（`grep '^define'` 里没有它）。
搜符号名时一定要分清这两者。

★ 另一个旁证：四个入口函数在 `-O` 汇编里的 `blr`（间接调用）出现 **0 次**。
对照第 7 章的 `dyn`（那里必然有 `ldr x1, [x1, #24]` + `br x1`）——
**静态分发下的 trait 方法调用没有任何额外代价**，因为 MIR 里就已经写明了是哪个 impl。

## 8.3 为什么必须这样设计

### 如果没有孤儿规则：一个两 crate 的推演

假设 `impl Display for Vec<u64>` 是允许的。那么：

- crate A 写一份，把 `[1,2]` 打印成 `"[1, 2]"`；
- crate B 也写一份，打印成 `"vec![1,2]"`；
- 你同时依赖 A 和 B。

现在 `v.to_string()` 该调哪个？**没有答案。**
更糟的是：这个冲突可能在你**什么都没改**的情况下出现——
只要升级一个依赖，构建就挂了。

Reference 对这个动机的表述（原文）：

> An orphan implementation is one that implements a foreign trait for a foreign
> type. If these were freely allowed, **two crates could implement the same trait
> for the same type in incompatible ways**, creating a situation where adding or
> updating a dependency could break compilation due to conflicting implementations.

实测（两 crate 探针，`upstream` 定义 `trait Format`，`downstream` 试图 blanket impl）
就是 8.0 的那段输出：**E0210，连门都进不去**。

这个探针**可以直接跑**：

```bash
bash examples/ch08-coherence/cross-crate/run.sh
```

★ 注意它为什么必须是个脚本、不能只用一条 `rustc`：

```bash
# ❌ 这样证明不了任何东西 —— 两个文件是**同一个 crate**
rustc --edition 2024 --crate-type=lib examples/ch08-coherence/cross-crate/downstream.rs
```

同一个 crate 里，`upstream` 的 trait 就是**本地的**，孤儿规则直接放行。
脚本的做法是编译两次：先把 `upstream.rs` 编成 rlib，
再用 `--extern` 把 `downstream.rs` 编成**另一个** crate。
只有第 2 步失败，才说明"跨 crate 时孤儿规则生效"。

### 为什么"负向推理"是禁区

上面说的是"两个 crate 抢同一个 impl"。但还有一类更隐蔽的冲突：
**上游未来可能加的 impl**。

```rust
// 你今天写这个（假设允许）
impl<T: Copy> P for T { ... }
```

编译器要判断它是否和别的 impl 冲突，就得推理
"哪些 `T` **不**满足 `Copy`"。而在一个开放的世界里，
**上游随时可以给任何类型加 `Copy` 的 impl**——今天的"不冲突"明天就冲突了。

这就是所谓**负向推理（negative reasoning）**：编译器不能把
"某个 trait 没有被实现"当成一个事实。Rust 里唯一的例外是**本地类型**——
因为孤儿规则保证了"只有当前 crate 能写 `Local<...>` 的 impl"，
所以"上游不会给 `Local<T>` 加 impl"是一个**可以依赖的负向事实**。

> 这就是 8.2.2 那条"覆盖规则"存在的全部理由：
> `impl<T> ForeignTrait for Local<T>` 之所以安全，
> 不是因为它看起来合理，而是因为**没有别人能写 `Local<T>` 的 impl**。

### 为什么 blanket impl 在本 crate 里合法

回到 8.0 那个"同一个写法，一个行一个不行"：

```rust
impl<T: std::fmt::Display> Format   for T { ... }   // ❌ trait 是外部的
impl<T: MyDisplay>        MyDebug   for T { ... }   // ✅ trait 是本地的
```

区别在**谁能加冲突的 impl**：

- `Format` 是上游的。上游（或任何其他 crate）都可以写
  `impl Format for SomeType`，于是你的 blanket 随时可能被撞上
  —— 而 blanket impl 的冲突是**全局**的，编译器只能直接拒绝；
- `MyDebug` 是你的。只有你能给它写 impl，所以"你的 blanket 会不会和别人的撞"
  这个问题根本不存在。

**所以 blanket impl 不是"高级技巧"，而是"拥有 trait"这件事带来的能力。**
想 blanket impl 别人的 trait，唯一的正道是：
定义自己的 trait（或者用 newtype 把类型变成自己的）。

## 8.4 反直觉的点

### 反直觉之一：孤儿规则保护的是**库作者**，不是编译器

大多数人的直觉是"这条规则在为难我"。但 Reference 把动机说得很清楚（原文）：

> The orphan rule enables library authors to add new implementations to their
> traits without fear that they'll break downstream code. Without these
> restrictions, a library couldn't add an implementation like
> `impl<T: Display> MyTrait for T` without potentially conflicting with
> downstream implementations.

**正是因为下游不能 blanket impl 上游的 trait，上游才能放心地自己加 blanket impl。**
标准库能写 `impl<T: Display> ToString for T`，靠的就是这条规则。

### 反直觉之二：newtype 不是"workaround"，它就是你要的那个类型

被拒绝时最常见的建议是"用 newtype 包一下"：

```rust
pub struct Wrapped(pub Vec<u64>);
impl std::fmt::Display for Wrapped { ... }
```

很多人把它当成"绕过编译器的技巧"。但从类型系统的角度看，
**`Wrapped` 和 `Vec<u64>` 本来就是不同的类型**：

- 它有不同的不变量（你可以给它加约束）；
- 它可以有自己的方法（不会和 `Vec` 的方法撞名）；
- 它可以让 `Display` 的输出有唯一确定的语义。

孤儿规则没有阻止你做任何事，它只是要求你**把"这是我的类型"写出来**。

### 反直觉之三：E0119 是**保守**的——它拒绝的代码可能是对的

`impl<T: Copy> P for T` 和 `impl<T: Clone> P for T`：
今天所有 `Copy` 都是 `Clone`，所以这两个 impl 确实会撞。

但如果约束是 `T: Copy` 和 `T: MyMarker`，而**没有任何类型同时满足两者**呢？
编译器**照样拒绝**。因为它不做负向推理，无法证明"没有类型同时满足"。

**coherence 是一个充分条件，不是一个必要条件。**
它宁可拒绝一些其实合法的代码，也不接受"今天合法、明天上游一改就崩"的风险。
这是 Rust 在"表达力"和"稳定性"之间的一次明确取舍。

### 反直觉之四：这一章的一切，运行期一个字都不剩

孤儿规则、覆盖规则、E0119、blanket impl 的展开——
**全部是编译期的判定**。实测：

```bash
grep -c 'my_debug' .evidence/ch08-coherence-lib.O3.s     # → 0（连符号都没有）
for f in use_my_display use_wrapped use_blanket use_fundamental; do
  awk "/^_$f:/,/cfi_endproc/" .evidence/ch08-coherence-lib.O3.s | grep -c blr   # → 0
done
```

没有查表、没有注册表、没有初始化、没有间接调用。
**coherence 是纯编译期的记账，代价为零。**

（这也是它和 `dyn` 的根本区别：第 7 章那些 vtable 是**运行期的数据**，
而这里的规则只在编译器里存在。）

## 8.5 亲手验证

```bash
tools/evidence.sh ch08-coherence
scripts/verify-all.sh ch08

# 反例逐个跑
for f in orphan conflicting_blanket uncovered_param orphan_vec overlapping_blanket; do
  rustc --edition 2024 --crate-type=lib examples/ch08-coherence/fail/$f.rs 2>&1 | head -3
done

# 人读的符号名（★ tools/objdump.sh 带了 --demangle）
tools/objdump.sh ch08-coherence 3 | grep -E '^[0-9a-f]+ <'
```

**怎么算验证成功**：

1. `orphan.rs` 报 **E0117**，且带
   `impl doesn't have any local type before any uncovered type parameters`；
2. `uncovered_param.rs` 报 **E0210**（`Vec<Local>` 不算覆盖）；
3. `orphan_vec.rs` 也报 **E0117**，而 `src/lib.rs` 里的 `Box<Local>` 编译通过
   —— 这一对就是 fundamental 的全部证据；
4. `overlapping_blanket.rs` 的 E0119 **不带** `for type`，
   而 `conflicting_blanket.rs` 的**带**；
5. MIR 里能看到 `<Vec<u64> as MyDebug>::my_debug` —— blanket impl 已单态化。

第 4 条对应的符号名（`--demangle` 之后）长这样：

```text
<<alloc::boxed::Box<lib::Local> as core::fmt::Display>::fmt>:
```

```bash
scripts/verify-all.sh ch08      # 11 条断言
```

## 8.6 与 unsafe 的关系

这一章跟 `unsafe` 关系很小——它是**类型层**的规则，不涉及任何运行时保证。
但有两个间接联系值得记住：

**(1) coherence 是 `unsafe` 代码"不用担心的那一半"。**
你自己写的 `unsafe` 抽象如果违反了安全契约，编译器不会帮你；
但 coherence 保证了"任何类型上的任何 trait 实现都是唯一的"——
所以你在 `unsafe` 代码里调用 `T::method()` 时，
**不会有第二个实现悄悄接管**。这是很多 `unsafe` 抽象能成立的前提。

**(2) `#[fundamental]` 是 `unsafe` 也不能碰的。**
你不能给自己的类型加 `#[fundamental]`（那是内部属性），
所以想"让 `MyWrapper<Local>` 在孤儿规则里算本地类型"是没有办法的。
这条路只有 newtype 一条。

**(3) 反过来，孤儿规则让"newtype 包装"成为一种 soundness 工具。**
标准库的很多 `unsafe` 抽象（比如各种 `#[repr(transparent)]` 包装）
之所以能安全地"借用"底层类型的行为，正是因为包装类型是**新的类型**——
它有自己独立的 trait 实现空间。

## 8.7 小结

- **孤儿规则的精确形式**：`impl<P1..=Pn> Trait<T1..=Tn> for T0` 合法，
  当且仅当「trait 是本地的」**或**「`T0..=Tn` 中第一个本地类型 `Ti`
  之前的范围里没有未覆盖的类型参数」。
- **"覆盖"不是"出现过"**：`Local<T>` 覆盖了 `T`，`Vec<Local>` 没有
  —— 所以 `impl<T> From<T> for Local<T>` ✅，`impl<T> From<T> for Vec<Local>` ❌。
- **`#[fundamental]` 让 `Box<T>` / `&T` 在孤儿规则里"透明"**：
  `Box<Local>` 算本地类型，`Vec<Local>` 不算。
- **E0119 有两种形态**：带 `for type X`（具体类型冲突）与不带（泛型层面重叠）。
  后者说明冲突的类型集合是无限的。
- **规则保护的是库作者**：正因为下游不能 blanket impl 上游的 trait，
  上游才能放心地自己加 blanket impl。
- **coherence 是保守的**：它拒绝一些其实合法的代码（不做负向推理），
  换取"升级依赖不会突然编译失败"。
- **代价为零**：全部是编译期判定，运行期没有符号、没有查表、没有间接调用。

下一章我们从"类型"转向"约束"：**HRTB**——
`for<'a>` 到底在量化什么，以及它为什么会让你的 `impl` 突然不满足约束。
