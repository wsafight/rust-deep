//! 第 1 章：借用检查器到底在检查什么
//!
//! 证据生成：tools/evidence.sh ch01-borrow
//!
//! 本章要讲的 MIR 样本**故意选得极小**：一个 `Point`、一次借用、一次写。
//! 这样读者能一眼读完整个函数体（29 行 MIR，单个基本块），
//! 把注意力放在"借用的存活区间"而不是 `vec!` 展开的噪音上。
//!
//! ⚠️ 反例（借用检查失败）放在 `fail/E0502.rs`，**不参与编译**——
//! 它编译不过，而且 `vec!` 的展开会把 MIR 淹没。
//! 反例的完整错误信息见 `evidence.md` 的「反例」一节。

/// 最小可读样本：借用的产生 → 使用 → 死亡，全部落在同一个基本块上。
///
/// 对应的 MIR（`rustc --edition 2024 --emit=mir --crate-type=lib`）：
/// ```text
/// bb0: {
///     _1 = Point { x: const 1f64, y: const 2f64 };
///     _2 = &_1;                  // ← 借用产生
///     _3 = copy ((*_2).0: f64);  // ← 最后一次使用 _2
///     (_1.0: f64) = const 3f64;  // ← 写 p.x：借用已死，合法
///     _4 = copy (_1.1: f64);
///     _0 = Add(copy _3, move _4);
///     return;
/// }
/// ```
/// 注意 `_1.0` 的写发生在 `_2` 最后一次被使用**之后** —— 这就是 NLL：
/// 存活区间由**使用点**决定，不由词法块决定。
pub fn simple() -> f64 {
    let mut p = Point { x: 1.0, y: 2.0 };
    let r = &p;          // ← 借用在这里"产生"
    let v = r.x;         // ← 最后一次使用 `r`
    p.x = 3.0;           // ← 因此这里写 p 是合法的
    v + p.y
}

pub struct Point { pub x: f64, pub y: f64 }

/// 第二个样本：**借用与可变借用交错**，证明"存活区间由使用点决定"。
///
/// 对应的 MIR（同一个基本块内）：
/// ```text
/// bb0: {
///     _1 = Counter { n: const 0_u64 };
///     _2 = &(_1.0: u64);          // ← 共享借用产生
///     _3 = copy (*_2);            // ← 最后一次使用 _2（存进 seen）
///     _5 = &mut _1;               // ← 可变借用：合法，因为 _2 已经死了
///     _4 = Counter::bump(move _5) -> ...;
/// }
/// ```
/// **把 `let seen = *r;` 删掉，同一份代码就报 E0502** ——
/// 变的是使用点的位置，不是词法作用域。
pub fn nll_ok() -> u64 {
    let mut c = Counter { n: 0 };
    let r = &c.n;        // 共享借用产生
    let seen = *r;       // ← 最后一次使用 r
    c.bump();            // ← 因此这里的可变借用合法
    seen + c.n
}

pub struct Counter { pub n: u64 }
impl Counter {
    pub fn bump(&mut self) { self.n += 1; }
}

/// 第三个样本：**路径敏感**——所有分支都用掉借用之后，冲突就消失了。
///
/// `if` / `else` 两个分支都使用了 `first`，所以走到 `if` 之后，
/// 没有任何路径还能到达使用点 → 借用已死 → `push` 合法。
///
/// 对照（见 `fail/use_after_conflict.rs`）：**只在一个分支里**使用，
/// 就会报 E0502 —— 因为"不走 if"那条路径仍然能到达使用点。
pub fn branch_all_use(flag: bool) -> usize {
    let mut v = vec![1u64, 2, 3];
    let first = &v[0];
    if flag {
        println!("{first}");
    } else {
        println!("{first}");
    }
    v.push(4);          // ← 合法：两条路径都在上面用掉了借用
    v.len()
}
