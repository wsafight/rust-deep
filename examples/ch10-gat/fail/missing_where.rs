// ⚠️ 故意编译不过：**GAT 的 `where Self: 'a` 是强制要求**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch10-gat/fail/missing_where.rs
//
// 预期：
//   error: missing required bound on `Item`
//     help: add the required where clause: `where Self: 'a`
//     = note: this bound is currently required to ensure that impls have maximum flexibility
//
// ★ 这条规则在 1.98 上**仍然是强制**的（编译器提示里直接带着
//   issue #87479 的链接，说明这是"临时强制、等待未来放宽"的状态）。
//
//   语义上的理由：`Self::Item<'a>` 只有在 `Self: 'a` 时才有定义 ——
//   否则"`Self` 里可能含有比 `'a` 更短命的东西"，
//   而 `Item<'a>` 又依赖 `Self`，循环就说不通了。

pub trait LendingIter {
    type Item<'a>;
    fn next(&mut self) -> Option<Self::Item<'_>>;
}

fn main() {}
