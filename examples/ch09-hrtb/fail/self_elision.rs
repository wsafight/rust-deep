// ⚠️ 故意编译不过：**HRTB 管的是 F，不是方法自己的签名**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch09-hrtb/fail/self_elision.rs
//
// 预期：
//   error: lifetime may not live long enough
//     method was supposed to return data with lifetime `'2`
//     but it is returning data with lifetime `'1`
//
// ★ 这里的 `F` 明明已经是 `for<'a> Fn(&'a str) -> &'a str`，
//   为什么 `parse` 还是编译不过？
//
//   因为**省略规则**遇到 `&self` 时，输出生命周期**一律取 `&self` 的那个**：
//
//     fn parse(&self, s: &str) -> &str
//     //       ^^^^^  '1        ^^^^ '1（被 &self 抢走了，不是 s 的）
//
//   所以返回值被绑在 `&self` 上，而不是 `s` 上。
//   而 `(self.0)(s)` 返回的是**借自 `s`** 的引用 —— 两者对不上。
//
//   ★ 关键区分：
//     - HRTB 约束的是 **`F` 这个类型**（"对每个 'a 都能把 &'a str 变成 &'a str"）；
//     - `parse` 自己的签名是**另一件事**，省略规则照常生效。
//
//   正确写法见 src/lib.rs：`pub fn parse<'a>(&self, s: &'a str) -> &'a str`。

pub struct Parser<F: for<'a> Fn(&'a str) -> &'a str>(pub F);

impl<F: for<'a> Fn(&'a str) -> &'a str> Parser<F> {
    pub fn parse(&self, s: &str) -> &str { (self.0)(s) }
}

fn main() {}
