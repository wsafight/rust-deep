// ⚠️ 故意编译不过：泛型方法让 trait 无法建 vtable
// 复现：rustc --edition 2024 --crate-type=lib examples/ch07-vtable/fail/not_dyn_compatible_generic.rs
//
// 预期：
//   error[E0038]: the trait `Bad` is not dyn compatible
//    note: for a trait to be dyn compatible it needs to allow building a vtable
//    note: ...because method `generic` has generic type parameters
//
// ★ 注意错误信息里的措辞："**needs to allow building a vtable**"。
//   这就是 dyn-compatibility（旧称 object safety）的本质：
//   **vtable 是一个固定大小的表，而泛型方法需要为每个 T 生成一个槽位** ——
//   表的大小无法确定，所以建不出来。

pub trait Bad {
    fn generic<T>(&self, t: T);
}

pub fn use_it(x: &dyn Bad) {}

fn main() {}
