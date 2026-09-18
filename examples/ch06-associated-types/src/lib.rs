//! 第 6 章：关联类型 vs 泛型参数
//!
//! 证据生成：tools/evidence.sh ch06-associated-types
//!
//! 核心判据只有一条：**一个类型能实现这个 trait 几次？**
//!   - 关联类型：**一次**（`Self::Item` 是 `Self` 的函数）
//!   - 泛型参数：**任意多次**（每个 `T` 一次）
//!
//! 这条判据决定了两件事：
//!   1. 能不能有多个实现（表达力）
//!   2. 调用点要不要写类型标注（推断难度）

// ---------- 关联类型：每个类型只能实现一次 ----------

pub trait Container {
    type Item;
    fn get(&self, i: usize) -> Option<&Self::Item>;
}

pub struct Numbers(pub Vec<u64>);

impl Container for Numbers {
    type Item = u64;
    fn get(&self, i: usize) -> Option<&u64> { self.0.get(i) }
}

/// ★ 关联类型的好处：**调用点不需要类型标注**
#[unsafe(no_mangle)]
pub fn use_container(c: &Numbers) -> u64 {
    *c.get(0).unwrap_or(&0)      // `Self::Item` 已经唯一确定
}

// ---------- 泛型参数：同一个类型可以实现多次 ----------

pub trait Conv<T> {
    fn conv(self) -> T;
}

pub struct Wrapper(pub u32);

impl Conv<u64> for Wrapper {
    fn conv(self) -> u64 { self.0 as u64 }
}
impl Conv<f64> for Wrapper {
    fn conv(self) -> f64 { self.0 as f64 }
}
impl Conv<String> for Wrapper {
    fn conv(self) -> String { self.0.to_string() }
}

/// ★ 泛型参数的代价：**调用点常常需要类型标注**
///
/// 下面这个能编译，是因为返回类型 `u64` 唯一确定了要用哪个 `impl`。
#[unsafe(no_mangle)]
pub fn conv_with_annotation(w: Wrapper) -> u64 {
    w.conv()
}

/// 但如果没有上下文能推断，就必须写标注 —— 见 `fail/ambiguous.rs`
#[unsafe(no_mangle)]
pub fn conv_explicit(w: Wrapper) -> f64 {
    Conv::<f64>::conv(w)
}
