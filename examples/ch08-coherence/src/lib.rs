//! 第 8 章：coherence、孤儿规则与 blanket impl
//!
//! 证据生成：tools/evidence.sh ch08-coherence
//!
//! 核心问题：**为什么我不能给别人的类型实现别人的 trait？**
//! 答案不是"设计者的洁癖"，而是**编译器必须保证"每个 (类型, trait) 对
//! 至多有一个 impl"** —— 否则方法调用就变成不可判定的了。
//!
//! 三条规则：
//!   1. **孤儿规则**：`impl 外部trait for 外部类型` 不行（至少一个是本地的）
//!   2. **coherence**：两个 impl 不能覆盖同一个 (类型, trait) 对
//!   3. **覆盖规则**：泛型 impl 里的类型参数必须出现在本地类型里
//!
//! ★ 还有一条容易被忽略的细节：**fundamental 类型**。
//!   `Box<T>`、`&T`、`&mut T` 被标记为 `#[fundamental]`，
//!   所以在孤儿规则眼里 `Box<Local>` / `&Local` **算本地类型**，
//!   而 `Vec<Local>` 不算。见 `fundamental` 一节与 `fail/orphan_vec.rs` 的对照。

// ---------- 本地 trait + 外部类型：允许 ----------

/// ★ 绕开孤儿规则的**正道**：定义自己的 trait
pub trait MyDisplay {
    fn my_fmt(&self) -> String;
}

impl MyDisplay for Vec<u64> {
    fn my_fmt(&self) -> String {
        format!("{self:?}")
    }
}

impl MyDisplay for String {
    fn my_fmt(&self) -> String {
        format!("str:{self}")
    }
}

#[unsafe(no_mangle)]
pub fn use_my_display() -> String {
    format!("{} {}", vec![1u64, 2].my_fmt(), String::from("hi").my_fmt())
}

// ---------- 外部 trait + 本地类型：允许 ----------

/// ★ 另一条正道：**newtype 包装**
///
/// 注意 `Display` 是外部的、`Wrapped` 是本地的 —— 所以合法。
/// 这是"新类型模式"（newtype pattern）的典型用法。
pub struct Wrapped(pub Vec<u64>);

impl std::fmt::Display for Wrapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

#[unsafe(no_mangle)]
pub fn use_wrapped() -> String {
    Wrapped(vec![1, 2, 3]).to_string()
}

// ---------- blanket impl：合法的写法 ----------

/// 给**所有**实现了 `MyDisplay` 的类型实现另一个 trait。
/// 这是合法的：因为 `MyDisplay` 是本地的。
pub trait MyDebug {
    fn my_debug(&self) -> String;
}

impl<T: MyDisplay> MyDebug for T {
    fn my_debug(&self) -> String {
        format!("MyDebug({})", self.my_fmt())
    }
}

#[unsafe(no_mangle)]
pub fn use_blanket() -> String {
    // ★ 这一行的 MIR 值得看一眼：`<Vec<u64> as MyDebug>::my_debug(...)`
    //   —— blanket impl 的 Self 在单态化后是**具体类型**，
    //   不是 `T`。单态化把 blanket impl 的"所有 T"收敛成了"这里用到的那个 T"。
    vec![1u64, 2].my_debug()
}

// ---------- fundamental 类型：Box / & 在孤儿规则里"透明" ----------

/// ★ `Box<T>` 被标记为 `#[fundamental]`：
/// 孤儿规则眼里 `Box<Local>` **就是**本地类型，所以可以给它实现外部 trait。
/// 对照：`Vec<Local>` 不行（`Vec` 不是 fundamental）——见 `fail/orphan_vec.rs`。
pub struct Local(pub u64);

impl std::fmt::Display for Box<Local> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Box<{}>", self.0)
    }
}

/// `&T` / `&mut T` 同样是 fundamental。注意这里还顺带展示了
/// **带参数的本地类型覆盖泛型参数**：`Local<T>` 让 `impl<T> From<T>` 合法
/// （`fail/uncovered_param.rs` 里 `Vec<Local>` 就不合法）。
pub struct Tagged<T>(pub T);

impl<T> From<T> for Tagged<T> {
    fn from(t: T) -> Self {
        Tagged(t)
    }
}

#[unsafe(no_mangle)]
pub fn use_fundamental() -> String {
    let b: Box<Local> = Box::new(Local(7));
    let t: Tagged<u64> = Tagged::from(7u64);
    format!("{b} {}", t.0)
}
