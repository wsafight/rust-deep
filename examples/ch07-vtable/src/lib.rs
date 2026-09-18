//! 第 7 章：dyn vs 泛型 —— vtable 布局与间接调用
//!
//! 证据生成：tools/evidence.sh ch07-vtable
//!
//! 注意：`&dyn Trait` **只作为参数**时不会生成任何 vtable —— 需要在某个地方
//! 真的发生 `Sq -> dyn Shape` 的 unsize 强制转换，vtable 才会落进 `.data` 段。
//! 下面的 `make_dyn` / `STATIC_VT` 就是这个构造点，第 7 章的 vtable 布局
//! 证据（size/align/drop/方法指针的物理排布）全部依赖它们。

pub trait Shape {
    fn area(&self) -> f64;
    /// 第二个方法：用来证明 vtable 里方法槽位是**按声明顺序**排列的
    fn name(&self) -> &'static str;
}

pub struct Sq(pub f64);
impl Shape for Sq {
    fn area(&self) -> f64 { self.0 * self.0 }
    fn name(&self) -> &'static str { "Sq" }
}

/// 动态分发：应从 vtable 偏移 24 取 `area` 的函数指针后尾跳转
pub fn dyn_area(s: &dyn Shape) -> f64 { s.area() }

/// 静态分发：应被完全内联为一次乘法
pub fn static_area(s: &Sq) -> f64 { s.area() }

/// ★ 构造点：强制 `Sq -> dyn Shape`，让 vtable 真正生成
#[inline(never)]
pub fn make_dyn(v: f64) -> Box<dyn Shape> { Box::new(Sq(v)) }

/// ★ 静态 vtable：`.data` 段里能直接读到 vtable 的完整内容
/// （`dyn Shape + Sync` 才能放进 `static`，这本身就是 Send/Sync 的一课）
pub static STATIC_VT: &(dyn Shape + Sync) = &Sq(2.0);

/// 同时调用两个方法：汇编里应看到 `#24`（area）与 `#32`（name）两个槽位
pub fn dyn_both(s: &dyn Shape) -> (f64, &'static str) { (s.area(), s.name()) }

// ---------- 静态分发 vs 动态分发：同一份逻辑 ----------

/// 静态分发：单态化，应被完全内联。
///
/// ⚠️ **不能加 `#[unsafe(no_mangle)]`**：泛型函数必须单态化，
/// 符号名必然带实例化信息。加了会得到
/// `warning: functions generic over types or consts must be mangled`。
/// 这是"泛型 = 单态化"的一个直接证据。
pub fn area_generic<T: Shape>(s: &T) -> f64 { s.area() }

/// ★ 调用点：`area_generic::<Sq>` 会被完全内联（见汇编里没有 `bl`）
#[unsafe(no_mangle)]
pub fn call_generic(s: &Sq) -> f64 { area_generic(s) }

/// 动态分发：走 vtable，应有间接调用
#[unsafe(no_mangle)]
pub fn area_dyn(s: &dyn Shape) -> f64 { s.area() }

/// 对照：`Box<dyn Shape>` 的调用（胖指针在 Box 里，多一次解引用）
#[unsafe(no_mangle)]
pub fn area_boxed(s: &Box<dyn Shape>) -> f64 { s.area() }
