// ⚠️ 故意编译不过：**rustc 有一个专门的 lint 抓这件事**（`invalid_reference_casting`）
// 复现：rustc --edition 2024 --crate-type=lib examples/ch25-aliasing/fail/write_through_shared_ref.rs
//
// 预期：
//   error: assigning to `&T` is undefined behavior, consider using an `UnsafeCell`
//     --> .../write_through_shared_ref.rs:32:14
//      |
//   30 |     let p = x as *const u64 as *mut u64;
//      |             --------------------------- casting happened here
//   32 |     unsafe { *p = v }
//      |              ^^^^^^
//      = note: `#[deny(invalid_reference_casting)]` on by default
//
// ★ 这条**不是** Miri 报的，是 **rustc 自己**报的 ——
//   而且这个 lint 是 **`deny` by default**（不是 warn）。
//
//   它抓的模式是"**把 `&T` 转成 `*mut T` 然后写**"。
//   注意 `casting happened here` 那行 —— rustc 把**铸型点**和**写入点**
//   一起标出来了，因为 UB 是这两件事**合起来**造成的。
//
// ★ 为什么借用检查器平时不管这件事？
//   因为这里全程没有越过借用检查器的规则：`&x` 是共享借用，`x` 一直活着，
//   没有冲突的 `&mut`。借用检查器看的是"引用存活区间有没有重叠"（第 1 章），
//   它**不追踪**你从引用派生出裸指针之后又做了什么。
//
//   所以 rustc 需要一条**单独的 lint** 来兜这件事 ——
//   而它只兜住了"同一函数内、铸型点和写入点都能看见"的情形。
//   跨函数、或者经由 `black_box` 的版本（见 `tests/aliasing.rs` 的 UB 用例）
//   就兜不住了，只能靠 **Miri**。
//
// ★ 正确做法：用 `UnsafeCell<T>`。它显式声明"这里允许通过共享引用修改"，
//   于是 `&UnsafeCell<T>` 不再是只读引用（见 `src/lib.rs` 的 `Cell2`）。

pub fn write_through_shared(x: &u64, v: u64) {
    // 把 &T 转成 *mut T —— 铸型本身合法，写入才越界
    let p = x as *const u64 as *mut u64;
    // SAFETY（伪）：这里其实不 sound —— &T 是只读的
    unsafe { *p = v }
}

pub fn demo() {
    let x = 0u64;
    write_through_shared(&x, 1);   // ← 通过共享引用写了
}

fn main() {}
