// ⚠️ 这个文件**故意编译不过**，不参与 workspace 构建。
// 用途：第 1 章的「反例」—— 展示借用检查器的错误信息结构。
//
// 复现：
//   rustc --edition 2024 --crate-type=lib examples/ch01-borrow/fail/E0502.rs
//
// 预期输出：
//   error[E0502]: cannot borrow `v` as mutable because it is also borrowed as immutable
//     |
//   3 |     let first = &v[0];
//     |                  - immutable borrow occurs here
//   4 |     v.push(4);
//     |                  ^^^^^^^^^ mutable borrow occurs here
//   5 |     println!("{first}");
//     |                ----- immutable borrow later used here
//
// ★ 错误信息里的三段标注，就是借用检查器眼中的三个关键点：
//   借用的产生点、冲突点、以及**最后一次使用点**（第三点正是 NLL 的核心）。

fn main() {
    let mut v = vec![1, 2, 3];
    let first = &v[0];     // 不可变借用产生
    v.push(4);             // 可变借用：冲突
    println!("{first}");   // 不可变借用在这里才最后一次被使用 → E0502
}
