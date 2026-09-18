# 第 3 章：协变、逆变与不变 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch03-variance
scripts/verify-all.sh ch03      # 7 条断言
```

## ⚠️ 本章证据链的特殊性（写作前必读）

**variance 没有可用的 dump 工具。** 实测：

| 手段 | 结果 |
|---|---|
| `-Zdump-variance` | ❌ **不存在**（`rustc -Z help` 里没有） |
| `#[rustc_variance]` 属性 | ❌ 报 `cannot find attribute rustc_variance in this scope` |
| `RUSTC_LOG=rustc_middle::ty::variance=debug` | ❌ release 构建里该 target 被静态禁用到 `info` 级别 |
| 看 LLVM IR / 汇编 | ❌ variance **不产生任何代码**（它只在类型检查期起作用） |

**所以本章唯一的证据是"能编译 / 不能编译"的对照。**
这也意味着本章的证据链**天然弱于第 1、2 章**——没有"可观察的产物"，
只有"编译器的判决"。写作时不要承诺"能看到 variance 的输出"。

## ★ 核心对照：同一个调用，协变过、不变不过

这是本章最干净的一组证据。

```rust
use std::cell::Cell;

pub struct CovHolder<'a> { pub r: &'a str }        // 协变
pub struct InvHolder<'a> { pub c: Cell<&'a str> }  // 不变

fn same_cov<'s>(h: CovHolder<'s>, s: &'s str) -> usize { h.r.len() + s.len() }
fn same_inv<'s>(h: InvHolder<'s>, s: &'s str) -> usize { h.c.get().len() + s.len() }

pub fn cov_ok() -> usize {
    let local = String::from("temp");
    let h: CovHolder<'static> = CovHolder { r: "static" };
    same_cov(h, &local)          // ✅ 'static 收缩成 local 的寿命
}

pub fn inv_err() -> usize {
    let local = String::from("temp");
    let h: InvHolder<'static> = InvHolder { c: Cell::new("static") };
    same_inv(h, &local)          // ❌ 'static 不能收缩
}
```

**唯一的区别是字段类型**：`&'a str` → `Cell<&'a str>`。
函数形状、调用形状、局部变量**完全一样**。

报错（`fail/invariance.rs`）：
```text
error[E0597]: `local` does not live long enough
  |     let h: InvHolder<'static> = InvHolder { c: Cell::new("static") };
  |            ------------------ type annotation requires that `local`
  |                               is borrowed for `'static`
  |     same_inv(h, &local)
  |                 ^^^^^^ borrowed value does not live long enough
```

**为什么 `Cell` 让类型变不变？**
因为 `Cell<&'a str>` 允许**通过共享引用写入**（`set` 只要 `&self`）。
如果 `Cell<&'static str>` 能收缩成 `Cell<&'short str>`，
那你就能往里塞一个短命引用 —— 而外面的代码仍然以为它装着 `'static` 的。
**不变性是"写权限"的代价。**

## ★ 逆变：`fn(T)` 在 `T` 上逆变

```rust
fn apply_static(f: TakesStatic) -> usize { f("y") }   // TakesStatic = fn(&'static str) -> usize
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

★ **`for<'a> fn(&'a _)` 这个写法就是 HRTB（高阶 trait bound）**，
第 9 章会专门讲。这里它出现在错误信息里，正好说明
"`fn(&str)`" 的完整含义是 "`for<'a> fn(&'a str)`"。

**方向**：
- **协变**：`'static` 是 `'a` 的**子类型** → `&'static T` 可以当 `&'a T` 用；
- **逆变**：子类型关系**翻转** → `fn(&'a T)` 是 `fn(&'static T)` 的子类型。

一句话记法：**"要求更宽的"可以替代"要求更窄的"。**
`fn(&'a str)` 要求更宽（任意生命周期），所以能替代 `fn(&'static str)`。

## ★ `&mut T`：在 `'a` 上协变，在 `T` 上不变

这是最容易搞混的一点，也是"`&mut` 为什么难搞"的根源。

| 类型 | 在生命周期上 | 在 T 上 |
|---|---|---|
| `&'a T` | 协变 | 协变 |
| `&'a mut T` | **协变** | **不变** |
| `Cell<T>` | —— | **不变** |
| `fn(T)` | —— | **逆变** |
| `fn() -> T` | —— | 协变 |

**"`&mut T` 不变"指的是 `T` 不变，不是 `'a` 不变。**
所以 `&mut &'static str` 不能当 `&mut &'a str` 用
（`T = &'static str` 不能收缩成 `&'a str`）。

正例（`mut_invariant_ok`）：`s` 的类型**没有**被收缩，所以合法。

## variance 是**推导**出来的

Rust 不会问你"这个类型是什么 variance"，它从结构推导：

- 内建规则：`&'a T` 协变；`&'a mut T` 在 `T` 上不变；`Cell<T>` 不变；
  `fn(T)` 逆变；`fn() -> T` 协变；`PhantomData<T>` 跟随 `T`；
- 组合规则：结构体的 variance = 各字段 variance 的**最小上界**
  （协变 ⊔ 不变 = 不变，等等）。

所以 `InvHolder<'a> { c: Cell<&'a str> }` 之所以不变，
是因为**它有一个不变的字段**。换个字段就换个 variance ——
这也解释了为什么加一个 `PhantomData<Cell<&'a ()>>` 就能把类型的 variance 改掉
（第 25 章会用这个技巧）。

## 待办

- [x] 断言 7 条全绿
- [ ] 补一个 `PhantomData` 改变 variance 的例子（为第 25 章埋伏笔）
- [ ] 第 4 章（自引用结构与 Pin 的前置知识）需要新 example
