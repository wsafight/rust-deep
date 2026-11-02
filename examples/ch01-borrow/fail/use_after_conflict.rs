// ⚠️ 这个文件**故意编译不过**，不参与 workspace 构建。
// 用途：第 1 章的「路径敏感」反例。
//
// 复现：
//   rustc --edition 2024 --crate-type=lib \
//         examples/ch01-borrow/fail/use_after_conflict.rs
//
// 预期输出：
//   error[E0502]: cannot borrow `v` as mutable because it is also borrowed as immutable
//     |
//   3 |     let first = &v[0];
//     |                  - immutable borrow occurs here
//   4 |     v.push(4);
//     |     ^^^^^^^^^ mutable borrow occurs here
//   5 |     if flag { println!("{first}"); }
//     |                              ----- immutable borrow later used here
//
// 对照 `branch_all_use` / `branch_one_use`：条件使用都位于 push 前，才能编译过。
// 关键是冲突与使用的先后顺序，不是使用引用的分支数量。

fn main() {
    let flag = std::env::args().len() > 1;
    let mut v = vec![1u64, 2, 3];
    let first = &v[0];
    v.push(4);                              // 冲突点
    if flag { println!("{first}"); }        // 修改之后仍使用旧引用 → E0502
    println!("{}", v.len());
}
