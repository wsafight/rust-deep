// ⚠️ 故意编译不过：**两个输入引用 + 输出引用 → 省略规则推不出来**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch20-async-lifetimes/fail/ambiguous_lifetime.rs
//
// 预期：
//   error[E0106]: missing lifetime specifier
//    = help: this function's return type contains a borrowed value, but the
//            signature does not say whether it is borrowed from `x` or `y`
//
// ★ 这条与第 2 章讲的是**同一条规则**（省略规则），
//   但 async 让它更容易踩：因为"返回的引用"在异步里看起来更像"状态机里的东西"，
//   人们会误以为可以省略。
//
// ★ 修法：显式写 `<'a>`（见 `src/lib.rs` 的 `longest`）。

pub async fn pick(x: &[u64], y: &[u64]) -> &u64 {
    std::future::ready(()).await;
    if x.len() > y.len() { &x[0] } else { &y[0] }
}

fn main() {}
