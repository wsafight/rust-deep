// ⚠️ 故意编译不过：**泛型参数没被本地类型覆盖**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch08-coherence/fail/uncovered_param.rs
//
// 预期：
//   error[E0210]: type parameter `T` must be used as an argument to some local type
//     = note: implementing a foreign trait is only possible if at least one of
//             the types for which it is implemented is local
//
// ★ `Local` 确实是本地类型，但它出现在 `Vec<Local>` 的**内部**，
//   而 `Vec` 是外部的。规则要求：**本地类型必须出现在"覆盖位置"**
//   （`impl 外部trait for 本地类型<...>` 或 `impl 外部trait for ...<本地类型>`）。
//   这条规则是为了"未来兼容"：如果今天允许，明天上游给 `Vec<T>` 加一个
//   同 trait 的 impl，你的代码就会突然冲突。

pub struct Local;

impl<T> From<T> for Vec<Local> {
    fn from(_: T) -> Self { Vec::new() }
}

fn main() {}
