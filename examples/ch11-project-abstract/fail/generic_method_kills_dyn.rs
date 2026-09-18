// ⚠️ 故意编译不过：**给 trait 加一个泛型方法，`dyn` 立刻失效**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch11-project-abstract/fail/generic_method_kills_dyn.rs
//
// 预期：
//   error[E0038]: the trait `Projection` is not dyn compatible
//     ...because method `project_as` has generic type parameters
//
// ★ 这是本章（第 11 章）要建立的核心直觉：
//   **每一处"更强的表达力"都在削弱"动态分发"。**
//
//   `src/lib.rs` 里的 `Projection` 只有关联类型 `Out`，
//   它是 dyn compatible 的（`sum_dyn` 能编译）。
//   这里加了一个泛型方法 `project_as<T>`，`dyn Projection` 立刻不可用。
//
//   理由（第 7 章）：vtable 是定长的，而 `project_as` 的每个 `T`
//   都需要一个槽位，`T` 有无穷多个。
//
//   ★ 而且注意：**关联类型不破坏 dyn compatibility，泛型方法才破坏。**
//   这是"用关联类型还是泛型参数"这个选择的一个隐藏权重。

pub trait Projection {
    type Out;
    fn project(&self, events: &[u64]) -> Self::Out;

    // 新需求："能不能顺便投影成别的类型？"
    fn project_as<T>(&self, events: &[u64]) -> T;
}

pub fn use_dyn(p: &dyn Projection<Out = u64>, e: &[u64]) -> u64 {
    p.project(e)
}

fn main() {}
