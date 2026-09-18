// ⚠️ 故意编译不过：**`drop(g)` 不能替代块作用域**（本章最反直觉的一条）
// 复现：rustc --edition 2024 --crate-type=lib examples/ch20-async-lifetimes/fail/drop_does_not_help.rs
//
// 预期：
//   error: future cannot be sent between threads safely
//   note: future is not `Send` as this value is used across an await
//     |
//   9 |     let g = m.lock().unwrap();
//     |         - has type `std::sync::MutexGuard<'_, u64>` which is not `Send`
//   ...
//  12 |     std::future::ready(()).await;
//     |                            ^^^^^ await occurs here, with `g` maybe used later
//
// ★ 注意这一行：**"with `g` maybe used later"** ——
//   明明上面已经 `drop(g)` 了，编译器还说它"可能稍后被用"。
//
//   为什么？因为编译器数的是**变量 `g` 的存活区间**（第 1 章），
//   不是**值的存活**。`drop(g)` 是一次**使用**，但它**没有缩短变量 `g` 的存活区间**。
//
// ★ 对照：把同样的代码收进一个块 `{ ... }`，future 立刻变成 `Send`
//   （见 `src/lib.rs` 的 `holds_guard_good`）。
//   区别只在于"块作用域让变量的存活区间在 `await` 之前结束"。
//
// ★ 实践结论：**要跨 `await` 就别让 `!Send` 的值进作用域**；
//   写 `drop()` 没有用，要写块。

use std::sync::Mutex;

pub async fn drop_does_not_help(m: &Mutex<u64>) -> u64 {
    let g = m.lock().unwrap();
    let v = *g;
    drop(g);                        // ← 看起来"用完了"，其实没有
    std::future::ready(()).await;   // ← 编译器仍然说 "g maybe used later"
    v
}

pub fn assert_send<F: Send>(_f: F) {}

pub fn check() {
    assert_send(drop_does_not_help(&Mutex::new(1)));   // ← 错误在这里报出
}

fn main() {}
