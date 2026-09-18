// ⚠️ 故意编译不过：返回值的生命周期比输入长
// 复现：rustc --edition 2024 --crate-type=lib examples/ch02-lifetimes/fail/outlives.rs
//
// 预期：error: lifetime may not live long enough
//   returning this value requires that `'a` must outlive `'static`
//
// ★ 这句话读作一个**不等式**：`'a: 'static`。
//   编译器在解约束，不是在量时间。

pub fn to_static<'a>(x: &'a str) -> &'static str { x }

fn main() {}
