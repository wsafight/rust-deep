// ⚠️ 故意编译不过：泛型参数导致**类型推断歧义**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch06-associated-types/fail/ambiguous.rs
//
// 预期：
//   error[E0283]: type annotations needed
//    note: multiple `impl`s satisfying `Wrapper: Conv<_>` found
//    help: consider giving `_y` an explicit type
//
// ★ 这正是"泛型参数"的代价：
//   一个类型可以有多个 impl，于是编译器必须**推断**用哪个。
//   推断不出来就报 E0283。
//
//   对照 `src/lib.rs` 的 `conv_with_annotation`：
//   那里返回类型 `u64` 唯一确定了 impl，所以不用标注。
//
// ★ 如果这里用的是**关联类型**，`Self::Item` 是唯一的，
//   根本不存在"用哪个 impl"这个问题 —— 也就不会有 E0283。

pub trait Conv<T> { fn conv(self) -> T; }

pub struct Wrapper(pub u32);

impl Conv<u64> for Wrapper { fn conv(self) -> u64 { self.0 as u64 } }
impl Conv<f64> for Wrapper { fn conv(self) -> f64 { self.0 as f64 } }

pub fn ambiguous() {
    let w = Wrapper(5);
    let _y = w.conv();      // ← E0283：不知道要 u64 还是 f64
}

fn main() {}
