// ⚠️ 故意编译不过：**blanket impl 与具体 impl 冲突**（第 8 章的判据）
// 复现：rustc --edition 2024 --crate-type=lib examples/ch11-project-abstract/fail/conflicting_blanket.rs
//
// 预期：
//   error[E0119]: conflicting implementations of trait `AllProjections` for type `Sum`
//
// ★ 场景：你想给**所有**类型都提供 `AllProjections`，
//   同时又想给 `Sum` 一个**特化**的版本。
//
//   不行 —— blanket impl 覆盖了 `Sum`，两者冲突。
//   （Rust 的 specialization 至今是 nightly-only，stable 上只能二选一。）
//
//   对照 `src/lib.rs` 里**合法**的 blanket impl：
//     `impl<T: Projection> Counted for T` —— 它合法是因为
//     `Counted` 是**本地的** trait（第 8 章：trait 是本地的 → 放行），
//     而且没有第二个 impl 去撞它。

pub trait AllProjections {
    fn project_all(&self, events: &[u64]) -> u64;
}

impl<T> AllProjections for T {
    fn project_all(&self, _events: &[u64]) -> u64 { 0 }
}

pub struct Sum;

impl AllProjections for Sum {
    fn project_all(&self, events: &[u64]) -> u64 { events.iter().sum() }
}

fn main() {}
