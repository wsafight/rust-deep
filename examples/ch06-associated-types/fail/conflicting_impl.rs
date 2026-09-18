// ⚠️ 故意编译不过：关联类型**只能实现一次**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch06-associated-types/fail/conflicting_impl.rs
//
// 预期：
//   error[E0119]: conflicting implementations of trait `Container` for type `Numbers`
//     first implementation here
//     conflicting implementation for `Numbers`
//
// ★ 这就是"关联类型"的定义：`Self::Item` 是 `Self` 的**函数**，
//   一个 `Self` 只能有一个 `Self::Item`。
//   想要"一个类型多个实现"，就必须用泛型参数。

pub trait Container {
    type Item;
    fn get(&self, i: usize) -> Option<&Self::Item>;
}

pub struct Numbers(pub Vec<u64>);

impl Container for Numbers {
    type Item = u64;
    fn get(&self, i: usize) -> Option<&u64> { self.0.get(i) }
}

// 第二次实现：E0119
impl Container for Numbers {
    type Item = String;
    fn get(&self, _i: usize) -> Option<&String> { None }
}

fn main() {}
