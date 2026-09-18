// ⚠️ 故意编译不过：返回 Self 让 trait 无法建 vtable
// 复现：rustc --edition 2024 --crate-type=lib examples/ch07-vtable/fail/not_dyn_compatible_self.rs
//
// 预期：
//   error[E0038]: the trait `Bad` is not dyn compatible
//    note: ...because method `clone_me` references the `Self` type in its return type
//
// ★ 为什么 `-> Self` 也不行？
//   因为 `dyn Bad` 的**大小未知**（`Self` 不是 `Sized`）。
//   vtable 里没法描述"返回一个大小未知的东西"。
//   —— 又是同一个理由：**vtable 是固定布局**。

pub trait Bad {
    fn clone_me(&self) -> Self;
}

pub fn use_it(x: &dyn Bad) {}

fn main() {}
