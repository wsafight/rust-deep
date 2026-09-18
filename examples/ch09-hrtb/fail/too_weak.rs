// ⚠️ 故意编译不过：**把生命周期提到函数签名 = 量化方向反了**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch09-hrtb/fail/too_weak.rs
//
// 预期：
//   error[E0597]: `local` does not live long enough
//     note: requirement that the value outlives `'a` introduced here
//
// ★ 对比 src/lib.rs 里的 `strong`（用 `for<'a>`）——它**编译通过**。
//   两者的唯一差别是量词的方向：
//
//   - `weak<'a, F: Fn(&'a str) -> &'a str>`：`'a` 由**调用者**选，
//     `f` 只能用活得 ≥ 'a 的字符串去调 → 函数内部的局部 String 不够长；
//   - `strong<F: for<'a> Fn(&'a str) -> &'a str>`：对**每一个** `'a` 都成立，
//     所以 `f` 可以接受任何生命周期的字符串，包括函数内部的局部 String。
//
//   这是 HRTB 最实用的判据：
//   **"我能不能用自己临时造的字符串去调这个闭包？"**
//   能 → 需要 HRTB；不能 → 普通的生命周期参数就够了。

pub fn weak<'a, F: Fn(&'a str) -> &'a str>(f: F, s: &'a str) -> usize {
    let local = String::from("local");
    // 这里两个调用都会失败：`local` 活不过 `'a`
    f(&local).len() + f(s).len()
}

fn main() {}
