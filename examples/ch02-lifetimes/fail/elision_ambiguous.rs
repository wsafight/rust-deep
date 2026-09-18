// ⚠️ 故意编译不过：省略规则不够用时必须显式标注
// 复现：rustc --edition 2024 --crate-type=lib examples/ch02-lifetimes/fail/elision_ambiguous.rs
//
// 预期：
//   error: lifetime may not live long enough
//     method was supposed to return data with lifetime `'2`
//     but it is returning data with lifetime `'1`
//
// ★ 错误信息里的 `'1` / `'2` 是**推断器给两个生命周期起的临时名字**，
//   不是你写的标注 —— 这正是"生命周期是求解出来的"的直接体现。

pub struct Parser<'a> { pub s: &'a str }

impl<'a> Parser<'a> {
    // 省略规则 2 说"输出跟 &self 走"，
    // 但函数体返回的是 other —— 于是冲突
    pub fn get2(&self, other: &str) -> &str { other }
}

fn main() {}
