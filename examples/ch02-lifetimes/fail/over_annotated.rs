// ⚠️ 故意编译不过：**过度标注**生命周期，反而让代码无法编译
// 复现：rustc --edition 2024 --crate-type=lib examples/ch02-lifetimes/fail/over_annotated.rs
//
// 预期：error[E0515]: cannot return value referencing local variable `local`
//
// ★ 对照 `src/lib.rs` 里的 `precise`：只让输出跟 x 绑，同样的函数体就能过。
//   结论：**标注写得越紧，求解空间越小，越容易被拒。**
//   生命周期标注不是"让编译器更宽容"，是"你主动承诺的约束"。

fn tied<'a>(x: &'a str, _y: &'a str) -> &'a str { x }

pub fn use_tied() -> &'static str {
    let local = String::from("temp");
    // 'a 同时约束 x 和 y，而 &local 只能满足很短的寿命
    // → 'a 被拉短到 local 的寿命 → 返回值不能是 'static
    tied("static str", &local)
}

fn main() {}
