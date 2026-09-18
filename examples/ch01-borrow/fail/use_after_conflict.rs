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
// ★ 对照 `src/lib.rs` 里的 `branch_all_use`：**两个分支都用**就能编译过。
//   区别不在作用域，而在"if 之后还有没有路径能到达使用点"。

fn main() {
    let flag = std::env::args().len() > 1;
    let mut v = vec![1u64, 2, 3];
    let first = &v[0];
    v.push(4);                              // 冲突点
    if flag { println!("{first}"); }        // 只有一个分支用借用 → E0502
    println!("{}", v.len());
}
