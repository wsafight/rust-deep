//! 第 3 章：协变、逆变与不变
//!
//! 证据生成：tools/evidence.sh ch03-variance
//!
//! ⚠️ **本章的证据层只有"能编译 / 不能编译"**：
//! variance 在 1.98 里**没有可用的 dump 工具**
//! （`-Zdump-variance` 不存在、`#[rustc_variance]` 已不对外可用、
//! `RUSTC_LOG` 的 variance target 在 release 构建里被静态禁用），
//! 而且 variance 本身**不产生任何代码**。
//!
//! 所以本章用「同一个形状的代码，一个过一个不过」来演示。
//! 反例在 `fail/`，正例在这里。

use std::cell::Cell;

// ---------- 三种 variance 的载体 ----------

/// **协变**：`&'a str` 在 `'a` 上协变（'static 可以当 'short 用）
pub struct CovHolder<'a> { pub r: &'a str }

/// **不变**：`Cell<&'a str>` 在 `'a` 上不变（Cell 内部可写）
pub struct InvHolder<'a> { pub c: Cell<&'a str> }

/// **逆变**的载体：函数指针的参数位置
pub type TakesStatic = fn(&'static str) -> usize;

// ---------- 协变：'static 可以收缩成 'short ----------

/// `CovHolder<'static>` 可以传进需要 `CovHolder<'s>` 的位置。
/// 对照 `fail/invariance.rs`：把 `CovHolder` 换成 `InvHolder` 就编译不过。
#[unsafe(no_mangle)]
pub fn cov_ok() -> usize {
    let local = String::from("temp");
    let h: CovHolder<'static> = CovHolder { r: "static" };
    same_cov(h, &local)
}

fn same_cov<'s>(h: CovHolder<'s>, s: &'s str) -> usize { h.r.len() + s.len() }

// ---------- 逆变：fn(&'long) 可以当 fn(&'short) 用 ----------

/// 一个"接受任意生命周期"的函数，当然也能接受 `'static`。
/// 这就是 `fn(&'a str)` 在参数位置上**逆变**的体现：
/// `fn(&'static str)` 是 `fn(&'a str)` 的**子类型**（因为 `'static` 是 `'a` 的子类型）。
#[unsafe(no_mangle)]
pub fn contrav_ok() -> usize { apply_static(any_lifetime) }

fn apply_static(f: TakesStatic) -> usize { f("y") }
fn any_lifetime<'a>(s: &'a str) -> usize { s.len() }

// ---------- 不变：&mut T 在 T 上不变 ----------

/// `&mut T` 在 `T` 上**不变**——所以 `&mut &'static str` 不能当 `&mut &'a str` 用。
/// 这正是"为什么 `&mut` 那么难搞"的根源。
/// 对照 `fail/invariance.rs` 的 `inv_err`。
#[unsafe(no_mangle)]
pub fn mut_invariant_ok() -> usize {
    let mut s: &'static str = "static";
    // 这里 s 的类型**没有**被收缩（'static 保持 'static），所以合法
    overwrite(&mut s, "another static");
    s.len()
}

fn overwrite<'a>(dst: &mut &'a str, src: &'a str) { *dst = src; }
