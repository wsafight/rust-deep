// ⚠️ 故意编译不过：**不变性**
// 复现：rustc --edition 2024 --crate-type=lib examples/ch03-variance/fail/invariance.rs
//
// 预期：error[E0597]: `local` does not live long enough
//       type annotation requires that `local` is borrowed for `'static`
//
// ★ 对照 `src/lib.rs` 的 `cov_ok`：
//   把 `CovHolder` 换成 `InvHolder`（字段从 `&'a str` 换成 `Cell<&'a str>`），
//   一模一样的调用就编译不过了。
//   **这就是"不变"的定义：不能把 'static 收缩成更短的生命周期。**

use std::cell::Cell;

pub struct CovHolder<'a> { pub r: &'a str }
pub struct InvHolder<'a> { pub c: Cell<&'a str> }

fn same_cov<'s>(h: CovHolder<'s>, s: &'s str) -> usize { h.r.len() + s.len() }
fn same_inv<'s>(h: InvHolder<'s>, s: &'s str) -> usize { h.c.get().len() + s.len() }

// 协变版本：编译过
pub fn cov_ok() -> usize {
    let local = String::from("temp");
    let h: CovHolder<'static> = CovHolder { r: "static" };
    same_cov(h, &local)
}

// 不变版本：编译不过
pub fn inv_err() -> usize {
    let local = String::from("temp");
    let h: InvHolder<'static> = InvHolder { c: Cell::new("static") };
    same_inv(h, &local)      // ← 'static 不能收缩成 local 的寿命
}

fn main() {}
