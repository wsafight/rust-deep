// ⚠️ 故意编译不过：**两个 blanket impl 在泛型层面重叠**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch08-coherence/fail/overlapping_blanket.rs
//
// 预期：
//   error[E0119]: conflicting implementations of trait `P`
//     first implementation here
//     conflicting implementation
//
// ★ 关键：报错**没有说"对哪个类型冲突"**（对比 conflicting_blanket.rs 里的
//   `for type u64`）。因为这两个 impl 覆盖的类型集合是**无限的**，
//   编译器报的是"泛型层面的重叠"。
//
//   `T: Copy` 和 `T: Clone` 今天确实有重叠（所有 Copy 都是 Clone），
//   但编译器甚至不做这个推理 —— 它只检查"两个 impl 的头部能不能同时匹配"。
//   这就是为什么 E0119 是**保守**的：它宁可拒绝也可能合法的代码。

pub trait P { fn p(&self) -> u64; }

impl<T: Copy> P for T { fn p(&self) -> u64 { 1 } }
impl<T: Clone> P for T { fn p(&self) -> u64 { 2 } }

fn main() {}
