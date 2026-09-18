//! 第 21 章：`async fn in trait` 的现状
//!
//! 证据生成：tools/evidence.sh ch21-afit
//!
//! 第 18–20 章把 `async` 从里到外拆了一遍。这一章换一个方向：
//! **当 `async fn` 出现在 trait 里**，前面那些机制会撞上什么。
//!
//! 1.98.1 的现状（实测）：
//!
//! | 你想做的 | 状态 |
//! |---|---|
//! | `trait T { async fn f(&self); }` | ✅ 稳定 |
//! | 泛型调用 `fn g<T: T>(t: &T)` | ✅ 稳定 |
//! | 表达"返回的 future 是 `Send`" | ❌ **做不到**（`async fn` 的返回值不透明） |
//! | `dyn T` | ❌ **不是 dyn compatible**（E0038） |
//! | 递归 `async fn` | ❌ E0733（第 18 章） |
//!
//! ★ 本章的核心矛盾（一句话）：
//!
//! > **`async fn` 在 trait 里隐藏了返回类型 ——
//! > 这既是它能用起来的原因，也是它不能表达 `Send`、不能 `dyn` 的原因。**
//!
//! 这条 lint 就是全部问题的浓缩：
//!
//! ```text
//! warning: use of `async fn` in public traits is discouraged as auto trait
//!          bounds cannot be specified
//!   = note: `#[warn(async_fn_in_trait)]` on by default
//! ```

#![allow(async_fn_in_trait)]

use std::future::Future;

// ============================================================
// 1) 基础：`async fn` 在 trait 里能用了
// ============================================================

/// ★ 1.98 上最直接的写法 —— 稳定。
///
/// 它等价于一个返回 `impl Future` 的方法（RPITIT），
/// 但**返回类型是不透明的**：调用者只知道它 `impl Future<Output = u64>`。
pub trait Store {
    async fn get(&self, k: u64) -> u64;
}

pub struct Mem;

impl Store for Mem {
    async fn get(&self, k: u64) -> u64 { k }
}

/// 泛型调用 —— **单态化**，与普通泛型一样（第 7 章）。
///
/// 每个 `S` 都会生成一份独立的 `use_store::<S>`，
/// 里面装着 `S::get` 的状态机。
// ⚠️ 泛型函数不能 `#[unsafe(no_mangle)]`（rustc 会警告：
//    "functions generic over types or consts must be mangled"）——
//    因为它会被单态化成多份。这本身就是"AFIT 走单态化"的旁证。
pub async fn use_store<S: Store>(s: &S) -> u64 {
    s.get(1).await
}

/// ★ 关键实测：`use_store::<Mem>` 是**单态化**的 ——
///
/// ```asm
/// __RNvCs..._3lib9use_storeINtCs..._3lib3MemEE
/// ```
///
/// 符号里带着 `Mem`。与第 7 章 `dyn` 的间接调用形成对照：
/// **AFIT 走的是单态化，没有 vtable。**
#[unsafe(no_mangle)]
pub async fn use_store_mem() -> u64 {
    use_store(&Mem).await
}

// ============================================================
// 2) ★ 核心问题：`async fn` 表达不了 `Send`
// ============================================================

/// ★ 这样写**编译不过**（见 `fail/afit_not_send.rs`）：
///
/// ```rust
/// pub trait StoreBad {
///     async fn get(&self, k: u64) -> u64;
/// }
///
/// pub async fn spawn_it<S: StoreBad + Send + Sync + 'static>(s: S) {
///     // 要求返回的 future 是 Send —— 但 trait 没承诺过
///     assert_send(async move { s.get(1).await });
/// }
/// ```
///
/// 报错：
///
/// ```text
/// error: future cannot be sent between threads safely
/// note: the trait bound `impl Future<Output = u64>: Send` is not satisfied
/// ```
///
/// ★ **为什么？** 因为 `async fn` 的返回类型是**不透明的** ——
/// trait 只承诺"它是一个 `Future`"，**没有承诺它是 `Send`**。
///
/// 而 `tokio::spawn` 恰恰要求 `F: Send + 'static`（第 22 章）。
/// 所以"用 `async fn` 写 trait + `tokio::spawn`"这个组合**直接撞墙**。

/// ★ 解法一：**用 RPITIT 手写，把 `Send` 写进签名**。
///
/// `impl Future<Output = u64> + Send` —— 这次返回类型**带上了 `Send`**。
/// 编译器会检查每个实现是否真的满足（比如实现里跨 `await` 持有
/// `Rc` 就会在这里报错，而不是在调用点 —— 第 20 章的传染问题被前移了）。
pub trait StoreSend {
    fn get(&self, k: u64) -> impl Future<Output = u64> + Send;
}

impl StoreSend for Mem {
    fn get(&self, k: u64) -> impl Future<Output = u64> + Send {
        async move { k }
    }
}

pub fn assert_send<F: Send>(_f: F) {}

/// 这次能过了 —— **因为 `Send` 写进了 trait 的签名**。
#[unsafe(no_mangle)]
pub fn spawn_ok(s: &Mem) {
    assert_send(StoreSend::get(s, 1));
}

/// ★ 解法一的代价：**每个实现都必须手写 `impl Future` 包装**，
/// 而且**不能再用 `async fn` 简写**（`async fn` 没法写 `+ Send`）。
///
/// 这就是 `async fn in trait` 最尴尬的地方：
/// **能用的时候不能用 `Send`，要用 `Send` 的时候不能用 `async fn`。**

/// ★ 解法二：**`Box<dyn Future>`** —— 回到 `dyn`，代价是一次堆分配。
pub trait StoreDyn {
    fn get(&self, k: u64) -> Box<dyn Future<Output = u64> + Send + '_>;
}

impl StoreDyn for Mem {
    fn get(&self, k: u64) -> Box<dyn Future<Output = u64> + Send + '_> {
        Box::new(async move { k })
    }
}

/// ★ 注意这里：`Box<dyn Future>` **不能直接 `.await`**（见
/// `fail/box_dyn_future_not_awaitable.rs`）：
///
/// ```text
/// error[E0277]: `dyn Future<Output = u64>` cannot be unpinned
/// ```
///
/// 原因：`.await` 的 blanket impl 要求 `F: Future + Unpin`（第 19 章），
/// 而 `dyn Future` **不是 `Unpin`**（trait object 默认 `!Unpin`）。
///
/// 修法：`Box::into_pin` 把它变成 `Pin<Box<dyn Future>>`。
#[unsafe(no_mangle)]
pub async fn use_store_dyn(s: &dyn StoreDyn) -> u64 {
    let f: Box<dyn Future<Output = u64> + Send + '_> = StoreDyn::get(s, 1);
    // `Box<dyn Future>` → `Pin<Box<dyn Future>>`：地址稳定，且 Pin 可用
    let mut f = Box::into_pin(f);
    std::future::poll_fn(|cx| f.as_mut().poll(cx)).await
}

// ============================================================
// 3) `dyn` 不兼容：AFIT 的 trait 建不出 vtable
// ============================================================

/// ★ 这样写**编译不过**（见 `fail/afit_not_dyn.rs`）：
///
/// ```rust
/// pub trait Store { async fn get(&self, k: u64) -> u64; }
/// pub fn make() -> Box<dyn Store> { todo!() }
/// ```
///
/// ```text
/// error[E0038]: the trait `Store` is not dyn compatible
///   = note: for a trait to be dyn compatible it needs to allow building a vtable
/// ```
///
/// ★ **为什么？** vtable 是一张**固定布局**的函数指针表（第 7 章）。
/// `async fn` 的返回类型对每个实现都**不同**（状态机类型不同）——
/// 而 vtable 只能放"签名相同"的函数指针。
///
/// 也就是说：**AFIT 和 `dyn` 在根上就冲突**，
/// 不是"还没实现"，是"按现在的 vtable 模型无法实现"。
///
/// 要 `dyn` 就必须回到 `Box<dyn Future>`（返回类型统一成"胖指针"）。

// ============================================================
// 4) 与第 20 章的交叉：`Send` 检查点被前移
// ============================================================

/// ★ 用 RPITIT 写 `+ Send` 的一个**副作用**：
/// `Send` 的检查发生在**实现处**，而不是调用点。
///
/// 下面是**编译不过**的（见 `fail/rpitit_send_at_impl.rs`）——
/// 注意错误指向的是 `impl` 里那几行，而不是任何调用点：
///
/// ```rust
/// pub struct Bad;
/// impl StoreSend for Bad {
///     fn get(&self, k: u64) -> impl Future<Output = u64> + Send {
///         async move {
///             let r = std::rc::Rc::new(k);   // ← Rc: !Send
///             std::future::ready(()).await;
///             *r
///         }
///     }
/// }
/// ```
///
/// ★ 这是**好事**：第 20 章讲的那个"`Send` 沿 `.await` 传染、
/// 报错位置离原因很远"的问题，在这里**被提前到了实现处**。
///
/// 代价是：实现者必须自己处理这个约束，
/// 而 `async fn` 简写**给不了这个选择**。

// ============================================================
// 5) 一张对照表（本章结论）
// ============================================================

/// | 写法 | 能 `dyn` | 能表达 `Send` | 分配 |
/// |---|---|---|---|
/// | `async fn` in trait | ❌ E0038 | ❌ | 无 |
/// | RPITIT `-> impl Future + Send` | ❌ | ✅ | 无 |
/// | `-> Box<dyn Future + Send>` | ✅ | ✅ | 一次堆分配 |
/// | `-> Pin<Box<dyn Future>>` | ✅ | ✅ | 一次堆分配 |
///
/// ★ **没有一列全是 ✅ 的行。** 这就是 1.98 上 AFIT 的现状：
/// 三个需求（`dyn` / `Send` / 零分配）**最多同时满足两个**。
#[unsafe(no_mangle)]
pub fn table_marker() {}
