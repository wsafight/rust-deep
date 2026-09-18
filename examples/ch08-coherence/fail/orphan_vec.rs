// ⚠️ 故意编译不过：**fundamental 类型的对照**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch08-coherence/fail/orphan_vec.rs
//
// 预期：
//   error[E0117]: only traits defined in the current crate can be implemented
//                  for types defined outside of the crate
//
// ★ 与 src/lib.rs 里的 `impl Display for Box<Local>` 逐字对照：
//   - `Box<T>` 是 `#[fundamental]` → `Box<Local>` 算本地类型 → ✅
//   - `Vec<T>` 不是 fundamental   → `Vec<Local>` 不算本地类型 → ❌
//
//   为什么 Box 要特殊？因为 Box 是"透明的所有权容器"——
//   它没有自己的语义，只是把值放到堆上。允许 `impl Trait for Box<Local>`
//   不会造成 coherence 问题（Box 不可能给 Local 加新的 impl）。
//   Vec 则不同：它是上游的、可能演进的类型。
//   fundamental 的完整名单见 `std` 源码里的 `#[fundamental]`（Box / &T / &mut T 等）。

pub struct Local(pub u64);

impl std::fmt::Display for Vec<Local> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.len())
    }
}

fn main() {}
