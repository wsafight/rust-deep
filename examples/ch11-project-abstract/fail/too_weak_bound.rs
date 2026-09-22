// ⚠️ 故意编译不过：**生命周期 bound 的作用域不对**（第 9 章的判据）
// 复现：rustc --edition 2024 --crate-type=lib examples/ch11-project-abstract/fail/too_weak_bound.rs
//
// 预期：
//   error[E0597]: `doubled` does not live long enough
//     note: requirement that the value outlives `'a` introduced here
//
// ★ 对照 src/lib.rs 里**正确**的 `run_pipeline`：
//   它写的是 `F: for<'a> Fn(&'a [u64]) -> u64` —— 编译通过。
//
//   这里把 `'a` 提到了函数签名上，于是 `'a` 由调用者选，
//   而流水线内部临时造的 `doubled` 活不过 `'a`。
//
//   ★ 这是第 9 章那条判据在真实 API 上的样子：
//     "我能不能用自己临时造的切片去调这个闭包？"
//     能 → 需要 `for<'a>`。

pub fn run_pipeline<'a, F>(events: &'a [u64], f: F) -> u64
where
    F: Fn(&'a [u64]) -> u64,
{
    let doubled: Vec<u64> = events.iter().map(|x| x * 2).collect();
    f(&doubled) + f(events)
}

fn main() {}
