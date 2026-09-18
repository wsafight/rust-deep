// ⚠️ 故意编译不过：**RPITIT 的 `+ Send` 让检查点前移到实现处**（这是好事）
// 复现：rustc --edition 2024 --crate-type=lib examples/ch21-afit/fail/rpitit_send_at_impl.rs
//
// 预期：
//   error: future cannot be sent between threads safely
//     = help: within `impl Future<Output = u64>`, the trait `Send` is not
//             implemented for `Rc<u64>`
//   note: future is not `Send` as this value is used across an await
//     --> 指向 **impl 里**的那几行（不是任何调用点）
//
// ★ 与 `afit_not_send.rs` 对比着看：
//
//   - 那边：错误报在**调用点**（因为 trait 没承诺 `Send`）；
//   - 这边：错误报在**实现处**（因为签名承诺了 `+ Send`，实现没做到）。
//
// ★ 这是**好事**：第 20 章讲的那个"`Send` 沿 `.await` 传染、
//   报错位置离原因很远"的问题，在这里**被提前到了原因所在的地方**。
//
// ★ 代价是：实现者必须自己处理这个约束，
//   而 `async fn` 简写**给不了这个选择** ——
//   要么用 `async fn`（写不了 `+ Send`），
//   要么手写 `impl Future`（每个实现都得写包装）。
//
//   这就是 1.98 上 AFIT 最尴尬的地方。

use std::future::Future;

pub trait StoreSend {
    fn get(&self, k: u64) -> impl Future<Output = u64> + Send;
}

pub struct Bad;

impl StoreSend for Bad {
    fn get(&self, k: u64) -> impl Future<Output = u64> + Send {
        async move {
            let r = std::rc::Rc::new(k);       // ← Rc: !Send
            std::future::ready(()).await;       // ← r 跨过 await
            *r
        }
    }
}

fn main() {}
