// 第 8 章 · 两 crate 探针 —— 下游（"你的 crate"）
//
// ⚠️ 故意编译不过：**下游无法 blanket impl 上游的 trait**
// 复现：examples/ch08-coherence/cross-crate/run.sh
//
// 预期：
//   error[E0210]: type parameter `T` must be used as an argument to some local type
//     = note: implementing a foreign trait is only possible if at least one of
//             the types for which it is implemented is local
//
// ★ 这就是 Rust Reference 里那句话的落地：
//   "The orphan rule enables library authors to add new implementations to their
//    traits without fear that they'll break downstream code."
//
//   注意：downstream 在这里**什么都没做错**——它只是想把上游的 trait
//   免费送给所有 Display 类型。但允许它就等于允许**任何** crate 这么做，
//   上游（或第三个 crate）再加一个 impl 就会全局冲突。
//
//   对照 src/lib.rs 里的 `impl<T: MyDisplay> MyDebug for T`：
//   一模一样的写法，只因为 trait 是本地的，就完全合法。

use upstream::Format;

impl<T: std::fmt::Display> Format for T {
    fn fmt_it(&self) -> String { format!("{self}") }
}
