# 3. 协变、逆变与不变

> 一句话：variance 是"子类型关系能不能传递"的规则。
> 它不产生任何代码，也没有 dump 工具——本章的证据只有编译器的判决。

## 先把语法认清

variance 不是新关键字，而是泛型类型对生命周期或类型参数的推导属性。
若 `'long: 'short`，`&'long T` 通常可当作 `&'short T` 使用；但
`&mut T` 在 `T` 上不变，因为通过它可以写回一个新值。函数参数位置则会
反转子类型方向，这就是逆变。

可以把 variance 理解成类型系统在问：**里面那件东西换成“活得更短的版本”，
外面的盒子还安全吗？**只读盒子通常敢换，可写盒子就得谨慎得多。

### 放到业务里：回调注册与长期配置

事件系统可能保存一个能处理**任意短期请求引用**的回调：

```rust
type Handler = for<'a> fn(&'a Request);
```

把“只能处理 `'static` 请求”的函数塞进去并不安全，因为真实请求通常只活到
一次调用结束。variance 决定这种替换能否成立，也决定含引用的缓存句柄、
构建器和 FFI wrapper 是否能安全缩短生命周期。

## 3.0 一个会让你卡住的例子

```rust
let s: &'static str = "literal";     // 活到程序结束
let local = String::from("temp");    // 只活到块结束

let r: &str = s;                     // ✅ 长的当短的用，没问题
```

再看这个：

```rust
let mut s: &'static str = "literal";
let local = String::from("temp");
let r: &mut &str = &mut s;
*r = &local;                         // ❌ 报错：`local` does not live long enough
```

两个 `&mut` 看起来一样，为什么一个行一个不行？

再换一个：

```rust
let c: Cell<&'static str> = Cell::new("literal");
c.set(&local);                       // ❌ 同样报错
```

`Cell` 跟 `&mut` 有什么共同点？

**这一章讲的就是这件事**：类型里的 `'a` 能不能"变短"，
取决于它**出现在什么位置上**。

## 3.1 先把常见说法摆上桌

通常先看到的是一张表：

- `&'a T`：在 `'a` 上**协变**，在 `T` 上**协变**；
- `&'a mut T`：在 `'a` 上协变，在 `T` 上**不变**；
- `Cell<T>` / `RefCell<T>` / `UnsafeCell<T>`：在 `T` 上**不变**；
- `fn(T)`：在 `T` 上**逆变**；`fn() -> T`：在 `T` 上协变。

这些都对。但它们是一张**表**，不是一个**理由**。
你要么背下来，要么理解为什么——这一章试图给你理由。

## 3.2 编译器眼里的样子

### 3.2.1 先说清楚本章的证据是什么

**variance 没有可观察的产物。** 实测确认：

| 手段 | 结果 |
|---|---|
| `-Zdump-variance` | ❌ **不存在**（`rustc -Z help` 里没有） |
| `#[rustc_variance]` | ❌ `cannot find attribute rustc_variance in this scope` |
| `RUSTC_LOG=...variance=debug` | ❌ release 构建里该 target 被静态禁用到 `info` |
| LLVM IR / 汇编 | ❌ variance **不产生任何代码** |

所以本章的证据只能是**编译器的判决**：这段代码编译过，那段不过。
**这是本章的诚实边界**——它比第 1、2 章弱，我们得说清楚。

### 3.2.2 最干净的一组对照

```rust
use std::cell::Cell;

pub struct CovHolder<'a> { pub r: &'a str }        // 协变
pub struct InvHolder<'a> { pub c: Cell<&'a str> }  // 不变

fn same_cov<'s>(h: CovHolder<'s>, s: &'s str) -> usize { h.r.len() + s.len() }
fn same_inv<'s>(h: InvHolder<'s>, s: &'s str) -> usize { h.c.get().len() + s.len() }

pub fn cov_ok() -> usize {
    let local = String::from("temp");
    let h: CovHolder<'static> = CovHolder { r: "static" };
    same_cov(h, &local)          // ✅ 编译过
}

pub fn inv_err() -> usize {
    let local = String::from("temp");
    let h: InvHolder<'static> = InvHolder { c: Cell::new("static") };
    same_inv(h, &local)          // ❌ 编译不过
}
```

**唯一的区别是字段类型**：`&'a str` → `Cell<&'a str>`。
函数形状、调用形状、局部变量**完全一样**。

```text
error[E0597]: `local` does not live long enough
  |     let h: InvHolder<'static> = InvHolder { c: Cell::new("static") };
  |            ------------------ type annotation requires that `local`
  |                               is borrowed for `'static`
  |     same_inv(h, &local)
  |                 ^^^^^^ borrowed value does not live long enough
```

**这就是"不变"的定义**：`InvHolder<'static>` **不能**收缩成 `InvHolder<'s>`，
即使 `'s` 比 `'static` 短。

### 3.2.3 为什么 `Cell` 让类型变不变

因为 `Cell<&'a str>` 允许**通过共享引用写入**——`set` 只需要 `&self`。

设想如果 `Cell<&'static str>` 能收缩成 `Cell<&'short str>`：

1. 你有一个 `c: Cell<&'static str>`，外面别的地方也拿到了 `&c`；
2. 通过收缩后的 `Cell<&'short str>`，你 `c.set(&local)` 塞进一个短命引用；
3. 但外部那些 `&Cell<&'static str>` 仍然以为里面装的是 `'static` 的东西——
   于是它们可以在 `local` 已经死了之后读出那个引用。

**不变性就是"写权限"的代价。**
`&mut T` 同理：`&mut` 允许写 `T`，所以 `T` 必须不变。

> 反过来，`&'a T` 在 `T` 上**是**协变的——因为 `&` 不允许写。
> **"能不能写"决定了"能不能变"。** 这一条比那张表有用得多。

### 3.2.4 逆变：`fn(T)` 在 `T` 上

```rust
type TakesStatic = fn(&'static str) -> usize;

fn apply_static(f: TakesStatic) -> usize { f("y") }
fn any_lifetime<'a>(s: &'a str) -> usize { s.len() }

pub fn contrav_ok() -> usize { apply_static(any_lifetime) }   // ✅
```

一个"接受任意生命周期"的函数，当然能当"接受 `'static`"的函数用。

反方向（`fail/contravariance.rs`）：

```text
error[E0308]: mismatched types
  = note: expected fn pointer `for<'a> fn(&'a _) -> _`
                found fn item `fn(&'static _) -> _ {only_static}`
```

★ **`for<'a> fn(&'a _)` 就是 HRTB（高阶 trait bound）**，第 9 章专门讲。
这里它出现在错误信息里，正好说明 `fn(&str)` 的完整含义是
`for<'a> fn(&'a str)`。

**方向**：
- **协变**：`'static` 是 `'a` 的**子类型** → `&'static T` 可以当 `&'a T` 用；
- **逆变**：子类型关系**翻转** → `fn(&'a T)` 是 `fn(&'static T)` 的子类型。

一句话记法：**"要求更宽的"可以替代"要求更窄的"。**

### 3.2.5 一张完整的表（含容易搞混的那一格）

| 类型 | 在生命周期上 | 在 `T` 上 |
|---|---|---|
| `&'a T` | 协变 | 协变 |
| `&'a mut T` | **协变** | **不变** |
| `Cell<T>` / `RefCell<T>` | —— | **不变** |
| `fn(T)` | —— | **逆变** |
| `fn() -> T` | —— | 协变 |
| `PhantomData<T>` | —— | 跟随 `T` |

**最容易搞混的是 `&'a mut T`**：它在 `'a` 上**是协变的**，
"不变"说的是 `T`。所以：

```rust
&mut &'static str   不能当→   &mut &'a str      // T 位置，不变
&'a mut u64         可以当→   &'short mut u64   // 'a 位置，协变
```

### 3.2.6 variance 是**推导**出来的，不是声明出来的

Rust 从结构推导 variance：

- **内建规则**：上面那张表；
- **组合规则**：结构体的 variance = 各字段 variance 的**最小上界**
  （协变 ⊔ 不变 = 不变；协变 ⊔ 逆变 = 不变）。

所以 `InvHolder<'a> { c: Cell<&'a str> }` 之所以不变，
是因为**它有一个不变的字段**——即使 `&'a str` 本身是协变的。

这个规则有个直接推论：**加一个 `PhantomData<Cell<&'a ()>>` 字段，
就能把一个协变类型变成不变**，而不用真的存一个 `Cell`。
第 25 章会用这个技巧。

## 3.3 为什么必须这样设计

回到"能不能写"这条主线。

Rust 的类型系统要保证的核心性质是：

> **如果 `x: &T`，那么在 `x` 的整个生命周期里，
> 通过任何途径读到的 `T` 都不会变。**

这条性质是 LLVM 优化的基础（第 24 章的 `noalias`、`!invariant.load`）。
要让它在子类型转换下仍然成立，就必须限制"哪些转换是安全的"：

- **只读位置**（`&T` 的 `T`、`fn() -> T` 的 `T`）：
  变短是安全的——短命的东西当长命的用会出问题，反过来不会。
  → **协变**。
- **可写位置**（`&mut T` 的 `T`、`Cell<T>` 的 `T`）：
  变短**不**安全——因为写进去的东西会被"以为它更长"的人读到。
  → **不变**。
- **消费位置**（`fn(T)` 的 `T`）：方向反过来。
  一个能处理"任意"`T` 的函数，当然也能处理"更具体"的 `T`。
  → **逆变**。

**variance 不是三条孤立的规则，是同一条健全性论证在三种位置上的表现。**

## 3.4 反直觉的点

### 反直觉之一：`&mut T` 在生命周期上是**协变**的

大多数人第一次听到"`&mut` 是不变的"会以为整个类型都不变。
不是。`&'a mut T` 在 `'a` 上协变，只有 `T` 不变。

```rust
fn shrink<'a, 's: 'a>(x: &'a mut u64) -> &'s mut u64 { x }   // ✅ 合法
```

因为"这个可变引用本身能活多久"变短，不影响它能写什么。

### 反直觉之二：加了 `PhantomData` 反而让类型**更**受限

```rust
struct A<'a> { r: &'a str }                          // 协变：宽松
struct B<'a> { r: &'a str, _p: PhantomData<Cell<&'a ()>> }   // 不变：严格
```

给一个**已经能用**的类型加个"什么都没存"的字段，反而让它接受更少的代码。
`PhantomData` 是"零大小的类型层开关"——它不占空间，但参与 variance 推导。

**实践含义**：设计库的公开类型时，variance 是 API 的一部分。
把类型设计得太"宽"（协变）可能不够安全；
太"严"（不变）又会让用户写不出代码。这跟第 8 章的 coherence 一样，
是**类型层 API 设计**的问题。

### 反直觉之三：variance 错了不会报"variance 错误"

如果你把一个类型设计成了错误的 variance，编译器**不会**说
"这个类型的 variance 不对"。它只会在**使用点**报一个看似无关的错误
（E0597 / E0308 / E0502），错误信息里也不一定提 variance。

**实践含义**：遇到"莫名其妙的生命周期错误"时，
一个值得检查的方向是**你手上的类型是不是不变的**。
`&mut`、`Cell`、`RefCell`、`Mutex`、`RwLock`、`AtomicXxx`
—— 任何"能通过共享引用写"的东西，都会把它包着的 `T` 变成不变。

## 3.5 亲手验证

```bash
# 正例（全部编译过）
tools/evidence.sh ch03-variance
scripts/verify-all.sh ch03

# 反例（预期失败）
rustc --edition 2024 --crate-type=lib examples/ch03-variance/fail/invariance.rs
#   → error[E0597]: `local` does not live long enough

rustc --edition 2024 --crate-type=lib examples/ch03-variance/fail/contravariance.rs
#   → error[E0308]: mismatched types
#     expected fn pointer `for<'a> fn(&'a _) -> _`
#     found fn item `fn(&'static _) -> _ {only_static}`
```

**怎么算验证成功**：

1. `src/lib.rs` 里三个正例（`cov_ok` / `contrav_ok` / `mut_invariant_ok`）
   都在 `.evidence/ch03-variance-lib.O3.s` 里有函数体；
2. `fail/invariance.rs` 报 E0597 —— 把 `InvHolder` 换成 `CovHolder` 就过；
3. `fail/contravariance.rs` 报 E0308，且**错误信息里出现 `for<'a>`**
   —— 那是 HRTB 的痕迹。

```bash
scripts/verify-all.sh ch03      # 7 条断言
```

## 3.6 与 unsafe 的关系

variance 是**类型层**的健全性保证。`unsafe` 可以绕过它：

```rust
// 用 transmute 把 'static 变成 'short（或反过来）
let short: &'short str = unsafe { std::mem::transmute::<&'static str, &'short str>(long) };
```

**这个方向（长→短）是安全的**，本来就是协变允许的。
危险的是反过来（短→长）：你把一个只活一小会儿的引用，
交给了一个以为它活到程序结束的地方。

而如果你在**不变**的位置上撒谎（比如给 `Cell<T>` 写一个
`unsafe impl` 让它表现得协变），你会直接踩到第 25 章的别名规则。

**记住那条主线**：variance 是"能不能写"的产物。
在 `unsafe` 里绕过它，就等于承诺"我保证没人会在错误的时间写"——
而这个承诺必须有完整的论证（第 24 章）。

## 3.7 小结

- **variance 是子类型关系的传递规则**，不是"生命周期长短"的规则。
- **它的唯一判据是"能不能写"**：
  只读位置协变、可写位置不变、消费位置逆变。
- **`&'a mut T` 在 `'a` 上协变，在 `T` 上不变**——"不变"说的是 `T`。
- **variance 是推导出来的**：结构体的 variance = 各字段 variance 的最小上界。
  加一个 `PhantomData<Cell<&'a ()>>` 就能把协变变成不变。
- **variance 没有 dump 工具，也不产生代码**。本章的证据只有编译器的判决，
  这是本书里证据链**最弱**的一章——我们如实标注了这一点。
- **variance 错了不会报 variance 错误**，只会在使用点报一个看似无关的错。
  遇到"莫名其妙"的生命周期问题时，先看你手上的类型是不是不变的。
- **`for<'a>` 会出现在错误信息里**——那是第 9 章 HRTB 的伏笔。

下一章：把"引用指向自己"这件事单独拎出来，
看它为什么天然困难——这是 `Pin` 存在的原因。
