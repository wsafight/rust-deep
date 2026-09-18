// ⚠️ 故意编译不过：**`Box<dyn Future>` 不能直接 `.await`**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch21-afit/fail/box_dyn_future_not_awaitable.rs
//
// 预期：
//   error[E0277]: `dyn Future<Output = u64>` cannot be unpinned
//     --> ...
//      = note: required for `Box<dyn Future<Output = u64>>` to implement `Unpin`
//
// ★ 这是第 19 章的知识在一个新地方冒出来：
//   `.await` 的 blanket impl 要求 `F: Future + Unpin`，
//   而 **trait object 默认 `!Unpin`**（vtable 里没有 Unpin 的信息）。
//
// ★ 修法：`Box::into_pin` —— 把 `Box<dyn Future>` 变成 `Pin<Box<dyn Future>>`。
//   这正是"`Pin` 用借用的可变性表达能不能移动"的又一个实例（第 19 章）：
//   你要么有 `Unpin`（可以随便移），要么就得 `Pin` 住。
//
// ★ 注意这个错误**恰好出现在最需要 `dyn` 的地方**（动态分发的 async trait），
//   于是"用 `dyn` 解决 AFIT 的 dyn 问题"这条路上，必然要过 `Pin` 这一关。

use std::future::Future;

pub trait StoreDyn {
    fn get(&self, k: u64) -> Box<dyn Future<Output = u64> + '_>;
}

pub struct Mem;
impl StoreDyn for Mem {
    fn get(&self, k: u64) -> Box<dyn Future<Output = u64> + '_> {
        Box::new(async move { k })
    }
}

/// ← 这一行的 `.await` 报 E0277
pub async fn use_dyn(s: &dyn StoreDyn) -> u64 {
    StoreDyn::get(s, 1).await
}

fn main() {}
