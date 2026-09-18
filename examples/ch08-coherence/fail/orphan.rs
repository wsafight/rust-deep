// ⚠️ 故意编译不过：**孤儿规则**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch08-coherence/fail/orphan.rs
//
// 预期：
//   error[E0117]: only traits defined in the current crate can be implemented
//                  for types defined outside of the crate
//     = note: impl doesn't have any local type before any uncovered type parameters
//     = note: define and implement a trait or new type instead
//
// ★ 为什么这条规则存在？
//   如果允许 `impl Display for Vec<u64>`，那么**任何** crate 都能写它。
//   两个 crate 各写一份 → 同一个 (Vec<u64>, Display) 有两个 impl →
//   `v.to_string()` 该调哪个？**不可判定。**
//   孤儿规则是"保证 impl 唯一性"的**充分条件**。

impl std::fmt::Display for Vec<u64> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

fn main() {}
