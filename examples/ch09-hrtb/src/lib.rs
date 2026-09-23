//! 第 9 章：高阶 trait bound（HRTB）—— `for<'a>` 到底在量化什么
//!
//! 证据生成：tools/evidence.sh ch09-hrtb
//!
//! 本章的核心：**`for<'a>` 是 trait bound 上的"全称量词"**，
//! 而不是一个生命周期标注。它说的是：
//!
//! > "对**每一个**生命周期 `'a`，这个类型都满足 `Trait<'a>`。"
//!
//! 与之相对，`fn f<'a, F: Fn(&'a str)>` 会在每次调用中选定一个 `'a`，
//! 函数体只能假设 `F` 对这一个 `'a` 成立。
//! **两者的差别不是风格，是 bound 的作用域。**

use std::marker::PhantomData;

// ---------- 1) 省略写法本身就是 HRTB ----------

/// ★ 关键事实：`Fn(&str) -> &str` 是 **elided HRTB**。
/// 省略规则把 `Fn(&str) -> &str` 展开成 `for<'a> Fn(&'a str) -> &'a str`，
/// **不是** `Fn(&'static str) -> &'static str`，也不是某个自由的 `'a`。
///
/// 下面两个函数除了写法不同，**约束完全一样** ——
/// `call_elided` 与 `call_explicit` 的汇编可以逐条对照。
pub fn elided<F: Fn(&str) -> &str>(f: F) -> usize {
    f("hello").len()
}

pub fn explicit<F: for<'a> Fn(&'a str) -> &'a str>(f: F) -> usize {
    f("hello").len()
}

/// 一个具体的 `F`，用来把上面两个泛型函数**单态化**出来。
fn id_str(s: &str) -> &str {
    s
}

#[unsafe(no_mangle)]
pub fn call_elided() -> usize {
    elided(id_str)
}

#[unsafe(no_mangle)]
pub fn call_explicit() -> usize {
    explicit(id_str)
}

// ---------- 2) 太弱的 bound：把生命周期提到函数参数 ----------

/// ⚠️ 与这个形状对照（它**编译不过**，见 `fail/too_weak.rs`）：
///
/// ```rust,ignore
/// pub fn weak<'a, F: Fn(&'a str) -> &'a str>(f: F, s: &'a str) -> usize { f(s).len() }
/// ```
///
/// `'a` 出现在函数签名上 = **由调用者选择**。于是 `f` 只能用
/// "活得和 `'a` 一样久"的字符串去调。
///
/// 正确写法是把 `'a` 从签名上**拿掉**，换成 `for<'a>` ——
/// bound 从"只支持本次调用的 `'a`"变成"对所有 `'a` 都成立"。
pub fn strong<F>(f: F) -> usize
where
    F: for<'a> Fn(&'a str) -> &'a str,
{
    // ★ 注意这个局部 `String`：它的生命周期**短于函数体**，
    // 也短于调用者能提供的任何 `'a`。
    // `weak` 那种写法在这里必然报 E0597，而 `strong` 能过 ——
    // 差别只在生命周期 bound 的作用域。
    let local = String::from("local");
    f(&local).len()
}

#[unsafe(no_mangle)]
pub fn call_strong() -> usize {
    strong(id_str)
}

// ---------- 3) 存进 struct：省略规则在这里帮不了你 ----------

/// `Parser` 的字段是一个"能把 `&'a str` 变成 `&'a str`"的闭包。
/// 用显式 HRTB 表达 —— 因为 struct 字段里**没有**"省略规则"
/// （省略规则只作用于函数签名）。
pub struct Parser<F: for<'a> Fn(&'a str) -> &'a str>(pub F);

impl<F: for<'a> Fn(&'a str) -> &'a str> Parser<F> {
    /// ★ 这里有一个**必须显式标注**的坑（实测）：
    ///
    /// 写成 `fn parse(&self, s: &str) -> &str` 会报
    /// `error: lifetime may not live long enough` ——
    /// 因为省略规则遇到 `&self` 时，**输出生命周期一律取 `&self` 的那个**。
    /// 于是返回值被绑在 `&self` 上，而不是 `s` 上。
    ///
    /// **HRTB 约束的是 `F`，不是 `parse` 自己的签名。**
    /// 量词在 `F` 上，返回值该怎么标还是得自己标。
    pub fn parse<'a>(&self, s: &'a str) -> &'a str {
        (self.0)(s)
    }
}

#[unsafe(no_mangle)]
pub fn use_parser() -> usize {
    let p = Parser(id_str);
    let owned = String::from("hello");
    p.parse(&owned).len()
}

// ---------- 4) HRTB 表达"省略规则表达不了"的 bound ----------

/// 带生命周期参数的 trait —— 省略规则**没有**为它定义简写形式。
pub trait Visitor<'a> {
    fn visit(&self, s: &'a str) -> usize;
}

/// 所以这里**必须**显式写 `for<'a>`：
/// "对每一个 `'a`，`T` 都实现 `Visitor<'a>`"。
pub fn total_visits<T>(t: &T) -> usize
where
    T: for<'a> Visitor<'a>,
{
    let local = String::from("abc");
    t.visit(&local) + t.visit("literal")
}

/// 一个具体实现，用来让上面的函数真的被单态化出来。
pub struct Len;

impl<'a> Visitor<'a> for Len {
    fn visit(&self, s: &'a str) -> usize {
        s.len()
    }
}

#[unsafe(no_mangle)]
pub fn call_visitor() -> usize {
    total_visits(&Len)
}

// ---------- 5) dyn 上的 HRTB ----------

/// trait object 也可以带 HRTB。
/// 注意 `Box<dyn for<'a> Fn(&'a str) -> &'a str>` 是**一个** trait object，
/// 而不是"对每个 'a 一个 trait object"。
pub fn boxed_parser() -> Box<dyn for<'a> Fn(&'a str) -> &'a str> {
    Box::new(id_str)
}

#[unsafe(no_mangle)]
pub fn call_boxed_parser() -> usize {
    boxed_parser()("hello").len()
}

// ---------- 6) 反面对照：把生命周期绑到 struct 上 ----------

/// 把生命周期参数放到 struct 上（而不是用 HRTB），`Fn(&'a str)` 就**合法**了。
/// 因为此时 `'a` 是**类型的一部分**，调用者需要显式给出 `Cb<'a, F>`。
///
/// 但代价是：`call` 只能接受"活得至少和 `'a` 一样久"的字符串 ——
/// 这就是 `fail/too_weak.rs` 里的 E0597。
pub struct Cb<'a, F: Fn(&'a str) -> usize>(pub F, pub PhantomData<&'a ()>);

impl<'a, F: Fn(&'a str) -> usize> Cb<'a, F> {
    pub fn call(&self, s: &'a str) -> usize {
        (self.0)(s)
    }
}

/// 用函数指针当 `F`（`fn(&'a str) -> usize` 是 `Fn(&'a str) -> usize` 的实现）。
#[unsafe(no_mangle)]
pub fn use_cb<'a>(c: &Cb<'a, fn(&'a str) -> usize>, s: &'a str) -> usize {
    c.call(s)
}
