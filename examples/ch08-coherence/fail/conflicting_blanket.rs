// ⚠️ 故意编译不过：**blanket impl 与具体 impl 冲突**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch08-coherence/fail/conflicting_blanket.rs
//
// 预期：
//   error[E0119]: conflicting implementations of trait `MyTrait` for type `u64`
//     first implementation here
//     conflicting implementation for `u64`
//
// ★ blanket impl 覆盖了**所有** T，包括 u64。
//   所以再给 u64 单独写一个就是"同一个 (u64, MyTrait) 对有两个 impl"。

pub trait MyTrait { fn f(&self) -> u64; }

impl<T> MyTrait for T { fn f(&self) -> u64 { 0 } }

impl MyTrait for u64 { fn f(&self) -> u64 { 1 } }

fn main() {}
