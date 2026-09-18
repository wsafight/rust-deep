//! 第 11 章（实战）：设计一个小型 trait 抽象层
//!
//! 证据生成：tools/evidence.sh ch11-project-abstract
//!
//! 本章把第 6–10 章的判据用在**同一个真实问题**上：
//!
//! > 设计一个"事件存储"（event store）抽象层：
//! > 能 append 事件、能按 key 读出投影。
//!
//! 问题本身很小，但它把五个判据全部逼出来了：
//!
//! | 判据 | 来自 | 在本章的落点 |
//! |---|---|---|
//! | 一个类型能实现几次 | 第 6 章 | `Store::Item` 用关联类型还是泛型参数 |
//! | 能不能建 vtable | 第 7 章 | `dyn Store` 能不能用 |
//! | 谁拥有 trait | 第 8 章 | 能不能给 `Vec<T>` 加 impl |
//! | 量化方向 | 第 9 章 | 投影函数要不要 `for<'a>` |
//! | 一族类型 | 第 10 章 | 读出来的投影要不要 GAT |
//!
//! ★ 结论先行：**五个判据互相牵制**。
//! 你在第 6 章为了"表达力"选了泛型参数，第 7 章就会发现 `dyn` 没了；
//! 你在第 10 章为了 lending 选了 GAT，第 7 章又会发现 `dyn` 没了。
//! **每一处"更强的表达力"都在削弱"动态分发"**——这是本章要建立的直觉。

use std::collections::HashMap;

// ============================================================
// 第 1 步：最小的抽象 —— 用关联类型（第 6 章的判据）
// ============================================================

/// 一个"读模型"：能从事件流里读出投影。
///
/// ★ 判据（第 6 章）：**一个 `Self` 对应几个投影类型？**
/// 一个 → 关联类型。
pub trait Projection {
    /// 投影出来的类型。每个实现只有一个 —— 所以用关联类型。
    type Out;

    fn project(&self, events: &[u64]) -> Self::Out;
}

/// 求和投影。
pub struct Sum;

impl Projection for Sum {
    type Out = u64;
    fn project(&self, events: &[u64]) -> u64 { events.iter().sum() }
}

/// 计数投影 —— 同一个 trait 的**另一个**实现。
///
/// ★ 注意：`Sum` 和 `Count` 是**不同的类型**，各自实现 `Projection`。
/// 这**不是**"一个类型实现多次"—— 所以关联类型完全够用。
pub struct Count;

impl Projection for Count {
    type Out = usize;
    fn project(&self, events: &[u64]) -> usize { events.len() }
}

#[unsafe(no_mangle)]
pub fn use_sum(events: &[u64]) -> u64 { Sum.project(events) }

#[unsafe(no_mangle)]
pub fn use_count(events: &[u64]) -> usize { Count.project(events) }

// ============================================================
// 第 2 步：`dyn` 版本 —— 第 7 章的判据
// ============================================================

/// `Projection` 是 **dyn compatible**：没有泛型方法、没有返回 `Self`、
/// 关联类型 `Out` 不参与方法签名（只在返回位置）。
///
/// ★ 所以这里**可以**建 vtable。这是设计的一个关键分水岭：
/// **关联类型不破坏 dyn compatibility，泛型方法才破坏。**
/// （加一个泛型方法就会变成 E0038 —— 见 `fail/generic_method_kills_dyn.rs`。）
pub fn sum_dyn(events: &[u64], p: &dyn Projection<Out = u64>) -> u64 {
    p.project(events)
}

/// ★★ 一个**必须**是 `no_mangle` + 参数来自外部 的版本 ——
/// 这样才能看到 `dyn` 的**真实代价**。
///
/// 实测发现：`use_dyn`（调用点传的是 `&Sum`）被 LLVM **完全去虚化**了 ——
/// 汇编里既没有 `ldr x?, [x?, #24]`，也没有 `br x?`。
/// 因为调用点的具体类型已知，vtable 加载被常量折叠掉了。
///
/// **`dyn` 只在"类型真的未知"时才付出代价。**
/// 下面这个函数从外部接收 `&dyn`，编译器无从知道是谁，
/// 于是必须走 vtable。
#[unsafe(no_mangle)]
pub fn sum_dyn_exported(events: &[u64], p: &dyn Projection<Out = u64>) -> u64 {
    p.project(events)
}

#[unsafe(no_mangle)]
pub fn use_dyn(events: &[u64]) -> u64 { sum_dyn(events, &Sum) }

// ============================================================
// 第 3 步：加一个"带 key"的投影 —— 逼出第 6 章的真正判据
// ============================================================

/// 现在需求变了：投影要按 key 分类（比如 `HashMap<K, u64>`）。
///
/// 这时"一个类型实现几次"的判据开始起作用：
/// 如果想让 `KeyedSum` 同时支持 `K = u64` 和 `K = String`，
/// 就必须**换成泛型参数**（或者用 GAT）。
///
/// 本章选 **GAT**：因为 key 类型是"一族"，而不是"一个"。
///
/// ★ 一条容易搞混的规则（本章实测）：
///   `where Self: 'a` 只有**生命周期参数**的 GAT 才需要。
///   这里参数是**类型** `K`，所以**不需要**任何 where 子句。
pub trait KeyedProjection {
    type Out<K>;

    fn project_keyed<K: Clone + Eq + std::hash::Hash>(
        &self,
        events: &[(K, u64)],
    ) -> Self::Out<K>;
}

/// 实现：按 key 求和，返回 `HashMap<K, u64>`。
pub struct KeyedSum;

impl KeyedProjection for KeyedSum {
    type Out<K> = HashMap<K, u64>;

    fn project_keyed<K: Clone + Eq + std::hash::Hash>(
        &self,
        events: &[(K, u64)],
    ) -> HashMap<K, u64> {
        let mut m = HashMap::new();
        for (k, v) in events {
            *m.entry(k.clone()).or_insert(0) += *v;
        }
        m
    }
}

#[unsafe(no_mangle)]
pub fn use_keyed_sum(events: &[(u64, u64)]) -> u64 {
    KeyedSum.project_keyed(events).values().copied().sum()
}

/// ★ 对照：如果**不用 GAT**，而是把 `K` 放到 trait 上：
///
/// ```rust,ignore
/// pub trait KeyedProjection2<K> { type Out; fn project_keyed(&self, events: &[(K, u64)]) -> Self::Out; }
/// ```
///
/// 那就得为每个 `K` 写一个 impl。`K` 是开放的，**写不完**。
/// 这就是第 10 章 GAT 在"类型参数"场景下的价值。
pub struct KeyedMax;

impl KeyedProjection for KeyedMax {
    type Out<K> = Option<(K, u64)>;

    fn project_keyed<K: Clone + Eq + std::hash::Hash>(
        &self,
        events: &[(K, u64)],
    ) -> Option<(K, u64)> {
        let mut best: Option<(K, u64)> = None;
        for (k, v) in events {
            match &best {
                Some((_, bv)) if *bv >= *v => {}
                _ => best = Some((k.clone(), *v)),
            }
        }
        best
    }
}

#[unsafe(no_mangle)]
pub fn use_keyed_max(events: &[(u64, u64)]) -> u64 {
    KeyedMax.project_keyed(events).map(|(_, v)| v).unwrap_or(0)
}

// ============================================================
// 第 4 步：投影要能"借出"内部数据 —— 第 10 章 + 第 9 章
// ============================================================

/// 需求再变：投影不想返回 `HashMap`（拷贝），而是想**借出**内部切片。
///
/// 这时就要 GAT + 生命周期参数，并且**必须**写 `where Self: 'a`。
pub trait BorrowingProjection {
    /// ★ 生命周期参数的 GAT：`where Self: 'a` 是**强制**的（第 10 章）。
    type Out<'a>
    where
        Self: 'a;

    fn project_borrowed<'a>(&'a self, events: &'a [u64]) -> Self::Out<'a>;
}

/// 一个"缓存投影"：内部存一份结果，借出它的切片。
pub struct CachedSum {
    cache: Vec<u64>,
}

impl CachedSum {
    pub fn new(events: &[u64]) -> Self {
        // 简单起见：把每个前缀和缓存起来
        let mut cache = Vec::with_capacity(events.len());
        let mut acc = 0u64;
        for e in events {
            acc = acc.wrapping_add(*e);
            cache.push(acc);
        }
        Self { cache }
    }
}

impl BorrowingProjection for CachedSum {
    /// ★ `Out<'a>` 借自 `self`（而不是借自 `events`）——
    /// 这正是第 10 章里 `Chunks` 那个例子的形状。
    type Out<'a>
        = &'a [u64]
    where
        Self: 'a;

    fn project_borrowed<'a>(&'a self, _events: &'a [u64]) -> &'a [u64] {
        &self.cache
    }
}

#[unsafe(no_mangle)]
pub fn use_borrowed(c: &CachedSum, events: &[u64]) -> u64 {
    c.project_borrowed(events).last().copied().unwrap_or(0)
}

// ============================================================
// 第 5 步：投影器要"能接受任意生命周期的输入" —— 第 9 章
// ============================================================

/// 一个"投影流水线"：对每个投影器调用一次。
///
/// ★ 这里需要 `for<'a>`（第 9 章）：
/// 闭包必须能接受**任意**生命周期的 `&[u64]`，
/// 因为流水线内部会传入**自己临时造的**切片。
pub fn run_pipeline<F>(events: &[u64], f: F) -> u64
where
    F: for<'a> Fn(&'a [u64]) -> u64,
{
    // ★ 关键：这个局部 `Vec` 的生命周期短于调用者能提供的任何 `'a`。
    // 如果约束写成 `F: Fn(&'a [u64]) -> u64`（'a 在签名上），
    // 这里就会报 E0597（见第 9 章）。
    let doubled: Vec<u64> = events.iter().map(|x| x * 2).collect();
    f(&doubled) + f(events)
}

#[unsafe(no_mangle)]
pub fn use_pipeline(events: &[u64]) -> u64 {
    run_pipeline(events, |ev| ev.iter().sum())
}

// ============================================================
// 第 6 步：把上面全部组装起来 —— 以及一个 coherence 的边界（第 8 章）
// ============================================================

/// 给**所有**实现了 `Projection` 的类型，自动提供"投影并计数"。
///
/// ★ 这是**合法**的 blanket impl，因为 `Counted` 是**本地的** trait
/// （第 8 章：trait 是本地的 → 放行）。
/// 如果 `Counted` 是别人的 trait，这里就是 E0210。
pub trait Counted {
    fn project_count(&self, events: &[u64]) -> usize;
}

impl<T: Projection> Counted for T {
    fn project_count(&self, events: &[u64]) -> usize {
        let _ = self.project(events);
        events.len()
    }
}

#[unsafe(no_mangle)]
pub fn use_counted(events: &[u64]) -> usize {
    // 注意这里：`Sum` 同时有 `Projection` 和 `Counted`（后者是 blanket 来的）
    Sum.project_count(events)
}

/// ★ 但**不能**给 `Projection` 加一个"对所有 `Vec<T>` 都适用"的 blanket impl
/// 然后再给具体类型写 —— 那是 E0119（第 8 章）。
/// 见 `fail/conflicting_blanket.rs`。
pub trait AllProjections {
    fn project_all(&self, events: &[u64]) -> u64;
}

impl<T> AllProjections for T {
    fn project_all(&self, _events: &[u64]) -> u64 { 0 }
}

#[unsafe(no_mangle)]
pub fn use_all(events: &[u64]) -> u64 { Sum.project_all(events) }
