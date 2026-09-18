// ⚠️ 故意编译不过：E0106 缺少生命周期标注
// 复现：rustc --edition 2024 --crate-type=lib examples/ch02-lifetimes/fail/missing_lifetime.rs
//
// 预期：
//   error[E0106]: missing lifetime specifier
//    = help: this function's return type contains a borrowed value,
//            but the signature does not say whether it is borrowed from `x` or `y`
//
// ★ 注意 help 的措辞："does not say whether it is borrowed from `x` or `y`"
//   —— 编译器不是在问"活多久"，而是在问"**跟着谁**"。
//   生命周期是**关系**（约束），不是时长。

fn longest(x: &str, y: &str) -> &str {
    if x.len() > y.len() { x } else { y }
}

fn main() { println!("{}", longest("a", "bb")); }
