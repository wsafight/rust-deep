// ⚠️ 故意编译不过：**带 `async fn` 的 trait 不是 dyn compatible**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch21-afit/fail/afit_not_dyn.rs
//
// 预期：
//   error[E0038]: the trait `Store` is not dyn compatible
//     --> ...
//      = note: for a trait to be dyn compatible it needs to allow building a vtable
//
// ★ 为什么？vtable 是一张**固定布局**的函数指针表（第 7 章：
//   前 3 个 slot 是 drop/size/align，方法从偏移 24 起）。
//
//   而 `async fn` 的返回类型对**每个实现都不同**（状态机类型不同）——
//   vtable 只能放"签名相同"的函数指针。
//
// ★ 所以这不是"还没实现"，而是**按现在的 vtable 模型无法实现**：
//   AFIT 和 `dyn` 在根上冲突。
//
// ★ 要 `dyn` 就必须回到 `Box<dyn Future>` ——
//   把返回类型统一成"胖指针"（见 `src/lib.rs` 的 `StoreDyn`）。

#![allow(async_fn_in_trait)]

pub trait Store {
    async fn get(&self, k: u64) -> u64;
}

pub struct Mem;
impl Store for Mem {
    async fn get(&self, k: u64) -> u64 { k }
}

/// ← 这一行报 E0038
pub fn make() -> Box<dyn Store> { Box::new(Mem) }

fn main() {}
