# 第 10 章：GAT（泛型关联类型）— 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> 本机工具链：`rustc 1.98.1` / `cargo 1.98.1` / LLVM 22.1.8。

## 复现命令

```bash
tools/evidence.sh ch10-gat       # 生成 .s / .ll / .mir / .o
scripts/verify-all.sh ch10       # 9 条断言（PASS=11）
```

## 关键结论与断言（9 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | GAT 让"借自 `self`"的迭代器可以编译 | MIR 里 `<Chunks as LendingIter>::next` |
| 2 | 借自外部 `'s` 的版本同样单态化 | MIR 里 `<Windows<'_> as LendingIter>::next` |
| 3 | GAT 的类型参数用法：一族类型 | MIR 里 `<Wrapper as Family>::wrap::<u64>` |
| 4 | GAT 迭代器版本存在 | `.O3.s` 的 `^_sum_windows:` |
| 5 | 手写循环对照版本存在 | `.O3.s` 的 `^_sum_windows_manual:` |
| 6 | 两者都被向量化（GAT 不引入额外开销） | `.O3.s` 里的 `add.2d` |
| 7 | 普通关联类型表达不了"借自 `self`" | `fail/no_gat.rs` → `lifetime may not live long enough` |
| 8 | `where Self: 'a` 是强制要求 | `fail/missing_where.rs` → `missing required bound on Item` |
| 9 | 带 GAT 的 trait 不是 dyn compatible | `fail/gat_not_dyn.rs` → **E0038** |

## ★ 核心证据：`Chunks`（数据是迭代器自己的）

```rust
pub struct Chunks { data: Vec<u64>, i: usize }

impl LendingIter for Chunks {
    type Item<'a> = &'a [u64] where Self: 'a;
    fn next(&mut self) -> Option<&[u64]> { ... }
}
```

这个例子的关键在于：**没有外部生命周期可用**。
`next` 返回的引用只能借自 `&mut self`。

不用 GAT 时，唯一能通过类型检查的写法是 `type Item = &'static [u64]`，
然后立刻被拒（`fail/no_gat.rs`）：

```text
error: lifetime may not live long enough
  --> examples/ch10-gat/fail/no_gat.rs:35:13
   |
31 |     fn next(&mut self) -> Option<Self::Item> {
   |             - let's call the lifetime of this reference `'1`
...
35 |             Some(w)
   |             ^^^^^^^ returning this value requires that `'1` must outlive `'static`
```

**这不是"编译器不够聪明"，是表达能力的硬边界：**
普通关联类型是 `Self -> Type` 的函数，而 lending iterator 需要
`Self -> ('a -> Type)`。后者就是 GAT。

MIR 里两边都完整单态化（`.evidence/ch10-gat-lib.mir`）：

```mir
_4 = <Windows<'_> as LendingIter>::next(move _5) -> [return: bb2, unwind continue];
_4 = <Chunks        as LendingIter>::next(move _5) -> [return: bb2, unwind: bb10];
```

★ 注意 `Chunks` 的 `next` 签名（第 4 行）里**完全没有 GAT 的痕迹**：

```mir
fn <impl at examples/ch10-gat/src/lib.rs:55:1: 55:28>::next(_1: &mut Chunks) -> Option<&[u64]>
```

**GAT 参数在单态化时就被代入了** —— 这和"生命周期被完全擦除"（第 2 章）
是同一类现象：编译期的量词在代码里不留痕迹。

## `where Self: 'a` 是强制要求（不是风格）

`fail/missing_where.rs`：

```rust
pub trait LendingIter {
    type Item<'a>;          // ← 漏了 where 子句
    fn next(&mut self) -> Option<Self::Item<'_>>;
}
```

```text
error: missing required bound on `Item`
  --> examples/ch10-gat/fail/missing_where.rs:17:5
   |
17 |     type Item<'a>;
   |     ^^^^^^^^^^^^^-
   |                  |
   |                  help: add the required where clause: `where Self: 'a`
   |
   = note: this bound is currently required to ensure that impls have maximum flexibility
   = note: we are soliciting feedback, see issue #87479 <https://github.com/rust-lang/rust/issues/87479>
```

★ 编译器**直接给出修复建议**，而且 note 里带着 issue 链接
（#87479）—— 说明这是"临时强制、等待未来放宽"的状态。
**在 1.98.1 上它仍然是强制的。**

## GAT 的代价：不再是 dyn compatible

`fail/gat_not_dyn.rs`：

```text
error[E0038]: the trait `LendingIter` is not dyn compatible
  --> examples/ch10-gat/fail/gat_not_dyn.rs:22:28
   |
22 | pub fn use_dyn(x: &mut dyn LendingIter) {
   |                            ^^^^^^^^^^^ `LendingIter` is not dyn compatible
   |
note: for a trait to be dyn compatible it needs to allow building a vtable
  --> examples/ch10-gat/fail/gat_not_dyn.rs:16:10
   |
16 |     type Item<'a>
   |          ^^^^ ...because it contains generic associated type `Item`
   = help: consider moving `Item` to another trait
```

**这和第 7 章的 E0038 是同一个理由**：vtable 是一张**定长**的表，
而 GAT 的参数数量不定（`'a` 可以有无穷多个取值），表长无法确定。

> 所以"用 GAT 表达 lending iterator"和"用 `dyn` 做动态分发"
> **不能同时要**。这是 GAT 最主要的实际限制。

## GAT vs trait 泛型参数

| | GAT | trait 泛型参数 |
|---|---|---|
| `Wrapper: Family` 有几个 impl | **1 个** | `T` 有多少个就多少 impl |
| 一个 impl 能覆盖几个 `T` | **所有** `T` | 只有那一个 |
| 能否 `dyn` | ❌ E0038 | ✅（如果没有 GAT 的话） |

用 GAT，**一个 impl 覆盖所有 `T`**（`impl Family for Wrapper { type Member<T> = Vec<T>; }`）；
用泛型参数，你得为每个 `T` 写一个 impl —— 而"所有 `T`"是写不完的。

## ★ 零成本：GAT 版本 ≈ 手写循环

`sum_windows`（用 GAT 迭代器）与 `sum_windows_manual`（纯手写 while 循环）
在 `-O` 下的对比：

| | 行数 | `add.2d` 条数 |
|---|---|---|
| `sum_windows`（GAT） | **61** | **11** |
| `sum_windows_manual`（手写） | **62** | **11** |

`diff` 之后只差一处：手写版本多一条 `ldr x11, [x0]`，
因为 LLVM 对两个版本选了**略微不同的向量化策略**
（一个用 `dup.2d v0, x11` 广播首元素，一个用重叠加载）。

```asm
; GAT 版本
_sum_windows:
	cmp	x1, #2
	b.hs	LBB3_2
	...
	add.2d	v0, v4, v0      ; ← 128-bit 向量化 + 4 路展开

; 手写版本
_sum_windows_manual:
	cmp	x1, #2
	b.hs	LBB4_2
	ldr	x11, [x0]       ; ← 唯一的差异：把 v[0] 提到循环外
	...
	add.2d	v5, v5, v0
```

**结论：GAT 不引入任何额外开销。** 61 vs 62 行、都是 11 条 SIMD 加法 ——
"用 trait 表达"和"手写循环"生成的是同一类代码。
差异来自 LLVM 的向量化启发式，不来自 GAT。

> ⚠️ 这里只说"指令数几乎相同"，**不说"哪个更快"** ——
> 本书在没有 benchmark 数据之前不写性能断言（见 PLAN §13 断点 6）。

## 交叉验证（可选）

```bash
# GAT 的实例化
grep -nE 'LendingIter|Family' .evidence/ch10-gat-lib.mir

# 零成本对照
diff <(awk '/^_sum_windows:/,/cfi_endproc/' .evidence/ch10-gat-lib.O3.s) \
     <(awk '/^_sum_windows_manual:/,/cfi_endproc/' .evidence/ch10-gat-lib.O3.s)
```
