// ⚠️ 故意编译不过：**省略规则表达不了带生命周期的 trait**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch09-hrtb/fail/no_hrtb_for_visitor.rs
//
// 预期：
//   error[E0597]: `local` does not live long enough
//     note: requirement that the value outlives `'a` introduced here
//
// ★ 对比 src/lib.rs 里的 `total_visits`：
//   那里写的是 `T: for<'a> Visitor<'a>` —— 显式 HRTB，编译通过。
//
//   这是"什么时候必须显式写 for<'a>"的典型场景：
//   **trait 本身带生命周期参数时，省略规则没有对应的简写形式。**
//   你只能：
//     (a) 把 `'a` 提升为函数的生命周期参数（= 太弱，就是本文件）；
//     (b) 写 `for<'a>`（= 正确，见 src/lib.rs）。
//
//   注意 `Visitor<'_>` 这种写法在这里直接报 E0637（`'_` 不能出现在这里）——
//   匿名生命周期**不能**用来表达"所有生命周期"。

pub trait Visitor<'a> {
    fn visit(&self, s: &'a str) -> usize;
}

// (a) 把 'a 提升成函数参数：`'a` 由调用者选，
//     于是函数内部的局部 String 活不过 `'a`。
pub fn total_visits<'a, T: Visitor<'a>>(t: &T) -> usize {
    let local = String::from("abc");
    t.visit(&local) + t.visit("literal")
}

fn main() {}
