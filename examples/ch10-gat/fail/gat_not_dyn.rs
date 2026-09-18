// ⚠️ 故意编译不过：**带 GAT 的 trait 不是 dyn compatible**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch10-gat/fail/gat_not_dyn.rs
//
// 预期：
//   error[E0038]: the trait `LendingIter` is not dyn compatible
//     ...because it contains generic associated type `Item`
//
// ★ 这和第 7 章的 E0038 是**同一个理由**：
//   vtable 是一张**定长**的表，而 GAT 的参数数量不定
//   （`'a` 可以有无穷多个取值），表长无法确定。
//
//   所以"用 GAT 表达 lending iterator"和"用 dyn 做动态分发"
//   这两件事**不能同时要**。这是 GAT 最主要的实际限制。

pub trait LendingIter {
    type Item<'a>
    where
        Self: 'a;
    fn next(&mut self) -> Option<Self::Item<'_>>;
}

pub fn use_dyn(x: &mut dyn LendingIter) {
    let _ = x.next();
}

fn main() {}
