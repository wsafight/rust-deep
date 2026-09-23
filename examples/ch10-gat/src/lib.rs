//! 第 10 章：GAT（泛型关联类型）
//!
//! 证据生成：tools/evidence.sh ch10-gat
//!
//! 本章的核心：**关联类型本来是"一个类型"，GAT 把它变成"一族类型"**
//! —— `type Item<'a>` 是 `Self` 的**函数** `'a -> Type`。
//!
//! 这解决了 `Iterator` 解决不了的那个问题：
//! `Iterator::Item` 是一个**固定**类型，所以 `next` 返回的东西
//! **不能借用 `self`**。而"借出内部数据的迭代器"（lending iterator）
//! 恰好需要这一点 —— 它的 `Item` 必须依赖 `next` 被调用时的那个 `'a`。

// ---------- 1) 不用 GAT：表达不出"借出内部数据" ----------

// 想写一个"每次返回窗口"的迭代器。
//
// 用普通关联类型只能写成这样 —— 而它**编译不过**（见 `fail/no_gat.rs`）：
//
// ```rust,ignore
// pub trait LendingIter {
//     type Item;
//     fn next(&mut self) -> Option<Self::Item>;   // ← Item 不能借用 self
// }
// ```
//
// 因为 `Item` 是**一个**固定类型，它不可能"随每次调用的生命周期变化"。
// 若 `Item = &'a [u64]`，那个 `'a` 必须来自**别处**（比如 struct 上的参数），
// 而不是 `&mut self` 的那个生命周期。

// ---------- 2) GAT 版本：`Item` 成为 `'a` 的函数 ----------

/// ★★ 最能说明问题的一个例子：**数据是迭代器自己拥有的**。
///
/// 此时连"借自某个外部 `'s`"这条退路都没有 ——
/// `next` 返回的引用**只能**借自 `&mut self` 本身。
///
/// 用普通关联类型写（见 `fail/no_gat.rs`），唯一能通过类型检查的写法是
/// `type Item = &'static [u64]`，然后立刻报：
///
/// ```text
/// error: lifetime may not live long enough
///   |
/// 12 |     fn next(&mut self) -> Option<Self::Item> {
///    |             - let's call the lifetime of this reference `'1`
/// 16 |             Some(w)
///    |             ^^^^^^^ returning this value requires that `'1` must outlive `'static`
/// ```
///
/// **"让 `Item` 活得比 `self` 还久"这件事，普通关联类型表达不了。**
pub struct Chunks {
    data: Vec<u64>,
    i: usize,
}

impl LendingIter for Chunks {
    /// GAT 的解法：`Item` 是 `'a` 的**函数**。
    /// 每次 `next` 借用 `self` 时，`'a` 取那个借用的生命周期。
    type Item<'a>
        = &'a [u64]
    where
        Self: 'a;

    fn next(&mut self) -> Option<&[u64]> {
        if self.i + 2 <= self.data.len() {
            let w = &self.data[self.i..self.i + 2];
            self.i += 1;
            Some(w)
        } else {
            None
        }
    }
}

/// ★ 关键语法：`type Item<'a> where Self: 'a;`
///
/// `where Self: 'a` 是**当前版本的强制要求**（不是可选的风格）：
/// 实测漏掉它会报
/// `error: missing required bound on `Item``，
/// 编译器甚至直接给出修复建议 `add the required where clause: `where Self: 'a``。
///
/// 语义：只有当 `Self` 活得比 `'a` 长时，`Item<'a>` 才有定义。
/// 这保证了 `Self::Item<'a>` 里不会出现"已经死了的 Self"。
pub trait LendingIter {
    type Item<'a>
    where
        Self: 'a;

    fn next(&mut self) -> Option<Self::Item<'_>>;
}

/// 具体实现：一个按窗口滑动的迭代器，每次借出 `&[u64]`。
pub struct Windows<'s> {
    s: &'s [u64],
    i: usize,
}

impl<'s> LendingIter for Windows<'s> {
    /// ★ 这里 `Item<'a>` 的具体类型是 `&'a [u64]` ——
    /// **借自 `self` 的数据**。普通关联类型做不到这一点。
    type Item<'a>
        = &'a [u64]
    where
        Self: 'a;

    fn next(&mut self) -> Option<&[u64]> {
        if self.i + 2 <= self.s.len() {
            let w = &self.s[self.i..self.i + 2];
            self.i += 1;
            Some(w)
        } else {
            None
        }
    }
}

/// 用 GAT 的迭代器：**每次调用返回的引用生命周期独立**
#[unsafe(no_mangle)]
pub fn sum_windows(v: &[u64]) -> u64 {
    let mut it = Windows { s: v, i: 0 };
    let mut acc = 0u64;
    while let Some(w) = it.next() {
        acc = acc.wrapping_add(w[0]).wrapping_add(w[1]);
    }
    acc
}

/// 拥有数据的版本 —— 唯一能编译的写法就是 GAT。
#[unsafe(no_mangle)]
pub fn sum_chunks(v: Vec<u64>) -> u64 {
    let mut it = Chunks { data: v, i: 0 };
    let mut acc = 0u64;
    while let Some(w) = it.next() {
        acc = acc.wrapping_add(w[0]).wrapping_add(w[1]);
    }
    acc
}

// ---------- 3) GAT 的另一种用法：一族类型（不只是生命周期） ----------

/// GAT 的参数不限于生命周期 —— 也可以是类型参数。
/// 这时它表达的是"**一族**类型"：对每个 `T` 给出一个成员类型。
pub trait Family {
    type Member<T>;

    fn wrap<T>(v: T) -> Self::Member<T>;
    fn unwrap<T>(m: Self::Member<T>) -> T;
}

/// `Wrapper` 的"一族类型"是 `Vec<T>`（对所有 `T`）。
pub struct Wrapper;

impl Family for Wrapper {
    type Member<T> = Vec<T>;

    fn wrap<T>(v: T) -> Vec<T> {
        vec![v]
    }
    fn unwrap<T>(m: Vec<T>) -> T {
        m.into_iter().next().unwrap()
    }
}

#[unsafe(no_mangle)]
pub fn use_family(v: u64) -> u64 {
    let m = <Wrapper as Family>::wrap(v);
    <Wrapper as Family>::unwrap(m)
}

// ---------- 4) 与"trait 泛型参数"的对照 ----------

/// 同样的意图，用 **trait 泛型参数**表达：
///
/// ```rust,ignore
/// pub trait Family2<T> { type Member; fn wrap(v: T) -> Self::Member; }
/// ```
///
/// 区别在于**关联类型的数量**：
///
/// | | GAT | trait 泛型参数 |
/// |---|---|---|
/// | `Wrapper: Family` 有几个 impl | **1 个** | `T` 有多少个就多少 impl |
/// | 一次 impl 里能表达几个 `T` | **所有** `T` | 只有那一个 |
/// | 能否 dyn | ❌ E0038 | ✅（如果没有 GAT 的话） |
///
/// 用 GAT，一个 impl 就覆盖了所有 `T`；
/// 用泛型参数，你得为每个 `T` 写一个 impl ——
/// 而"所有 `T`"是写不完的。
pub struct Wrapper2;

impl Family2 for Wrapper2 {
    type Member<T> = Vec<T>;

    fn wrap<T>(v: T) -> Vec<T> {
        vec![v]
    }
}

/// 对照用的 trait：注意它**没有**泛型参数，但成员类型是泛型的。
pub trait Family2 {
    type Member<T>;

    fn wrap<T>(v: T) -> Self::Member<T>;
}

#[unsafe(no_mangle)]
pub fn use_family2(v: u64) -> u64 {
    let m = <Wrapper2 as Family2>::wrap(v);
    m.into_iter().next().unwrap()
}

// ---------- 5) 零成本：GAT 完全单态化 ----------

/// GAT 的参数在编译期就被代入了，**运行期没有任何"类型表"**。
/// `sum_windows` 与手写的循环生成的代码应当是同构的。
///
/// 对照的手写版本（不用 trait）：
#[unsafe(no_mangle)]
pub fn sum_windows_manual(v: &[u64]) -> u64 {
    let mut acc = 0u64;
    let mut i = 0usize;
    while i + 2 <= v.len() {
        acc = acc.wrapping_add(v[i]).wrapping_add(v[i + 1]);
        i += 1;
    }
    acc
}
