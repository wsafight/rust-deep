// ⚠️ 故意编译不过：**`async fn` 不能递归**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch18-future/fail/recursive_async.rs
//
// 预期：
//   error[E0733]: recursion in an async fn requires boxing
//     = note: a recursive `async fn` call must introduce indirection such as
//             `Box::pin` to avoid an infinitely sized future
//
// ★ 这个错误**直接证明了"状态机大小"这个概念的存在**：
//   编译器说的理由是 "to avoid an infinitely sized future" ——
//   递归调用要求"状态机里装一个同类型的状态机"，大小无穷大。
//
//   修法是 `Box::pin`：把内层的大小变成一个指针（8 字节）。
//   这也是 `async` 递归唯一的写法（`async-recursion` crate 做的事）。

pub async fn rec(n: u64) -> u64 {
    if n == 0 {
        0
    } else {
        rec(n - 1).await + 1
    }
}

fn main() {}
