//! 第 2 章：生命周期 —— 标注、省略与推断
//!
//! 证据生成：tools/evidence.sh ch02-lifetimes
//!
//! 本章的核心：**生命周期不是"时间"，是约束**。
//! 编译器解一组不等式（`'a: 'b`），而不是在时间轴上量长度。
//! 最硬的证据：**生命周期在 LLVM IR 与汇编里完全不存在**。

/// 显式标注：`'a` 把两个参数和返回值绑在同一个约束上
#[unsafe(no_mangle)]
pub fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}

/// 省略版本：编译成**完全相同**的代码 —— `'a` 不影响 codegen
#[unsafe(no_mangle)]
pub fn longest_elided<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}

/// ★ 生命周期擦除的直接证据：这两个函数的 LLVM IR / 汇编**逐字节相同**
#[unsafe(no_mangle)]
#[allow(clippy::needless_lifetimes)]
pub fn with_lifetime<'a>(x: &'a str) -> usize {
    x.len()
}

#[unsafe(no_mangle)]
pub fn without_lifetime(x: &str) -> usize {
    x.len()
}

/// 省略规则 1：只有一个输入生命周期 → 输出用它
#[unsafe(no_mangle)]
pub fn elide_one(x: &str) -> &str {
    x
}

/// 省略规则 2：有 `&self` → 输出用 `self` 的生命周期
pub struct Parser<'a> {
    pub s: &'a str,
}

impl<'a> Parser<'a> {
    /// 省略成 `fn get<'s>(&'s self) -> &'s str`
    #[unsafe(no_mangle)]
    pub fn get(&self) -> &str {
        self.s
    }
}

/// 约束是**不等式**，不是时间轴：只要 `'b: 'a`（`'b` 比 `'a` 长）就成立
#[unsafe(no_mangle)]
pub fn outlives<'a, 'b: 'a>(x: &'a str, _y: &'b str) -> &'a str {
    x
}

/// ★ 对照 `fail/over_annotated.rs`：**精确**标注 —— 输出只跟 `x` 绑。
///
/// 两个函数体完全一样，只是 `_y` 不参与 `'a` 的约束。
/// 于是 `'a` 可以保持 `'static`，`use_precise` 能编译过。
///
/// **教训**：标注写得越紧（约束越多），求解空间越小，越容易被拒。
/// 省略规则给出的通常就是最宽松的那个。
#[unsafe(no_mangle)]
pub fn precise<'a>(x: &'a str, _y: &str) -> &'a str {
    x
}

#[unsafe(no_mangle)]
pub fn use_precise() -> &'static str {
    let local = String::from("temp");
    precise("static str", &local)
}
