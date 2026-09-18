// ⚠️ 故意编译不过：**逆变的方向搞反了**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch03-variance/fail/contravariance.rs
//
// 预期：error[E0308]: mismatched types
//       note: expected fn pointer `for<'a> fn(&'a _) -> _`
//             found fn item `fn(&'static _) -> _ {only_static}`
//
// ★ `fn(T)` 在 T 上**逆变**：
//   - `fn(&'a str)`（能接受任意生命周期）可以当 `fn(&'static str)` 用 ✅
//   - `fn(&'static str)`（只能接受 'static）**不能**当 `fn(&'a str)` 用 ❌
//   "要求更宽的"可以替代"要求更窄的" —— 方向与协变相反。

fn apply_any(f: fn(&str) -> usize) -> usize { f("y") }

fn only_static(s: &'static str) -> usize { s.len() }

pub fn contrav_err() -> usize {
    // only_static 只接受 'static，但 apply_any 会传给它任意生命周期的引用
    apply_any(only_static)
}

fn main() {}
