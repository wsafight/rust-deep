// ⚠️ 故意编译不过：**普通关联类型表达不了"借自 self"**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch10-gat/fail/no_gat.rs
//
// 预期：
//   error: lifetime may not live long enough
//     returning this value requires that `'1` must outlive `'static`
//
// ★ 场景：迭代器**自己拥有**数据（`Vec<u64>`），`next` 想借出内部的一个窗口。
//   返回的引用只能借自 `&mut self`，而 `Item` 是一个**固定的**类型，
//   它没法"随每次调用的生命周期变化"。
//
//   于是唯一能通过类型检查的写法是 `type Item = &'static [u64]` ——
//   而它立刻被拒绝：`self` 活得没有 `'static` 长。
//
//   这不是"编译器不够聪明"，而是**表达能力**的硬边界：
//   普通关联类型是 `Self -> Type` 的函数，不是 `Self -> ('a -> Type)`。
//   后者才是 GAT（见 src/lib.rs 的 `LendingIter`）。

pub struct Chunks {
    data: Vec<u64>,
    i: usize,
}

pub trait LendingIter {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;
}

impl LendingIter for Chunks {
    type Item = &'static [u64];
    fn next(&mut self) -> Option<Self::Item> {
        if self.i + 2 <= self.data.len() {
            let w = &self.data[self.i..self.i + 2];
            self.i += 1;
            Some(w)
        } else {
            None
        }
    }
}

fn main() {}
