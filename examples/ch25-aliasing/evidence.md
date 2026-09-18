# 第 25 章：别名规则、`UnsafeCell` 与 `PhantomData` — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> Miri：`miri 0.1.0 (923c95cdf5 2026-09-16)`（`rustc 1.100.0-nightly`）

## ★ 本章的证据层：主证据来自 Miri，不是 MIR

| 层 | 工具 | 承担什么 | 命令 |
|---|---|---|---|
| **主** | **Miri** | Stacked Borrows 的**真实执行语义** | `scripts/verify-miri.sh`（7 条） |
| 辅 | LLVM IR | 契约（`noalias` / `captures`） | `ch24-noalias` |
| 澄清 | MIR | 只用来**破除误解**：`no_retag` 不是 retag | `verify-all.sh ch25`（5 条） |

## ⚠️ 必须讲明白的坑：stable 的 MIR **看不到 retag 本体**

### 实测事实

| 写法 | `--emit=mir` 里出现什么 |
|---|---|
| `pub fn m(a: &mut u64)`（顶层引用参数） | ❌ 什么都不出现 |
| `pub fn f(a: &mut u64, b: &mut u64)` | ❌ 什么都不出现 |
| `pub fn first<'a>(v: &'a [u64]) -> &'a u64` | ❌ 什么都不出现 |
| `let p = (a,); let q = black_box(p); **q.0` | ✅ `no_retag` |
| `&Box<u64>` 解引用、`&mut &mut u64` 重借用 | ✅ `no_retag` |

**看起来"有 retag 的地方反而不打印"，很反直觉。**

### 机制（读 rustc 1.98 源码确认）

1. `Rvalue::Use` 带一个 `WithRetag::Yes/No` 枚举
   （`rustc_middle/src/mir/syntax.rs:488,1340`）。
2. pretty printer **只在 `No` 时**打印 `no_retag`
   （`rustc_middle/src/mir/pretty.rs:1138`，源码注释原文：
   *"With retag is more common so we only print when it's without."*）。
3. → **`--emit=mir` 里的 `no_retag` = "这里故意不做 retag"**，
   而**做了 retag 的地方是静默的**（`WithRetag::Yes` 不打印任何东西）。
4. stable 上 `no_retag` 只有两个来源：
   - `EraseDerefTemps`：把 `CopyForDeref` 改写成 `Use(.., WithRetag::No)`，
     源码注释 *"We do **NOT** want a retag here!"*；
   - `DerefSeparator`。

### 更关键：`-Zdump-mir` 也看不到 retag

对 `fn f(a: &mut u64, b: &mut u64)` dump **全部 pass**，**0 处 retag**。
1.98 的 pass 列表里**根本没有 AddRetag** —— **retag 是 codegen 阶段的事**。

**证据**：`-Zcodegen-emit-retag`（需 nightly/bootstrap）会让汇编里出现
`bl ___rust_retag_reg`（实测：`&mut` 参数 3 处、嵌套引用 4 处）。

> ⚠️ 初版结论（"retag 在借用检查阶段被折叠"）**归因是错的**，
> 2026-09-18 逐 pass 复核后修正。**"我在输出里看到了 X"不等于"X 在这里发生"** ——
> 要确认一个机制在哪里发生，得读**源码**，而不是读输出。

## ★ 核心证据一：`UnsafeCell` 是唯一合法的"通过共享引用修改"

```rust
pub struct Cell2<T> { inner: UnsafeCell<T> }
impl<T: Copy> Cell2<T> {
    pub fn get(&self) -> T { unsafe { *self.inner.get() } }
    pub fn set(&self, v: T) { unsafe { *self.inner.get() = v } }
}
```

- 有 `UnsafeCell` → **Miri 通过**（`tests/aliasing.rs::unsafe_cell_allows_shared_write`）；
- 没有 `UnsafeCell`（自己把 `&T` 转 `*mut T`）→ **rustc 直接拒绝**：

```text
error: assigning to `&T` is undefined behavior, consider using an `UnsafeCell`
  --> examples/ch25-aliasing/fail/write_through_shared_ref.rs:32:14
   |
30 |     let p = x as *const u64 as *mut u64;
   |             --------------------------- casting happened here
32 |     unsafe { *p = v }
   |              ^^^^^^
   = note: `#[deny(invalid_reference_casting)]` on by default
```

★ **这条不是 Miri 报的，是 rustc 自己报的**，而且是 **`deny` by default**。
它把**铸型点**和**写入点**一起标出来 —— UB 是这两件事**合起来**造成的。

★ 但它是**局部的**：只看得到同一函数体内的铸型 → 写入链。
跨函数 + `black_box` 就抓不到（见下方 UB 用例二）。

## ★ 核心证据二：顺序决定一切（本章最微妙的一条）

三种顺序，实测 Miri：

| 顺序 | 结果 |
|---|---|
| ① 建 `&T` → ② 通过 `UnsafeCell` 写 → ③ 用 `&T` | ❌ **UB** |
| ② 通过 `UnsafeCell` 写 → ① 建 `&T` | ✅ 合法 |
| 全程只用 `UnsafeCell` 指针，从不建 `&T` | ✅ 合法 |

```rust
// ❌ UB（tests/aliasing_ub.rs::shared_ref_invalidated_by_unsafe_cell_write）
let r: &u64 = unsafe { &*c.get() };      // ① 先建 &T
w.set(2);                                 // ② 经由 UnsafeCell 写（本身合法）
assert_eq!(*r, 2);                        // ③ 用 → UB

// ✅ 合法（tests/aliasing.rs::write_then_read_is_fine）
w.set(2);                                 // ① 先写
let r: &u64 = unsafe { &*c.get() };      // ② 后建 &T
assert_eq!(*r, 2);
```

Miri 的报错：

```text
error: Undefined Behavior: trying to retag from <N> for SharedReadOnly permission
       at alloc<N>[0x0], but that tag does not exist in the borrow stack
  = help: <N> was created by a SharedReadOnly retag at offsets [0x0..0x8]
  = help: <N> was later invalidated at offsets [0x0..0x8] by a write access
```

★ **`UnsafeCell` 让"那次写入"合法，但这次写入仍然会弹掉在此之前建立的 `&T`。**
**实践规则：不要在写入之前建立 `&T` 并持有到写入之后。**

## ★ 核心证据三：`PhantomData` 让裸指针携带权限

```rust
pub struct SharedReadOnly<'a, T> {
    ptr: *const T,
    _p: PhantomData<&'a T>,                  // 只读
}
pub struct SharedReadWrite<'a, T> {
    ptr: *mut T,
    _p: PhantomData<&'a UnsafeCell<T>>,      // 可写
}
```

**两个结构体的字段布局完全一样**（8 字节指针 + ZST），
区别只在 `PhantomData` 的**类型参数**。

`PhantomData` 是零大小的，但让类型参与三件事：

| 参与 | 说明 | 章 |
|---|---|---|
| 生命周期检查 | 指针借用了 `'a` | 本章 |
| auto trait 推导 | `PhantomData<*const T>` 让类型 `!Send` | 12 |
| variance | `PhantomData<&'a T>` 协变 | 3 |

★ **`PhantomData` 在 `unsafe` 代码里几乎总是必须的，不是可选的** ——
裸指针丢掉了上面三样东西，它是唯一的补法。

## ★ 核心证据四：`no_retag` 的样子（只用来说明误解）

```
$ grep no_retag .evidence/ch25-aliasing-lib.mir
        _2 = no_retag copy ((*_1).0: *const T);
        _2 = no_retag copy ((*_1).0: *mut T);
        _3 = no_retag copy ((*_1).0: *mut T);
        _4 = no_retag copy (_3.0: &&u64);
        _5 = no_retag copy (*_4);
```

前三条来自 `SharedReadOnly::get` / `SharedReadWrite::get` / `set`
（`PhantomData` 是 ZST，读取指针字段经 `EraseDerefTemps` 改写）；
后两条来自 `through_tuple`（`&&u64` 经 tuple + `black_box` 传递）。

**这些 `no_retag` 恰恰是"这里不做 retag"的标记** —— 不要当成 retag 的证据。

## Miri 用例清单（`scripts/verify-miri.sh`，7 条全绿）

| 目标 | 语义 | 用例数 |
|---|---|---|
| `ch25-aliasing/sb_legal` | 必须**通过** | 3 |
| `ch25-aliasing/sb_ub` | 必须**报 UB** | 2 |
| `ch25-aliasing/aliasing` | 必须**通过** | 8 |
| `ch25-aliasing/aliasing_ub` | 必须**报 UB** | 4 |
| `ch19-pin/selfref` | 必须**通过** | 4 |
| `ch19-pin/selfref_ub` | 必须**报 UB** | 2 |

### 四个 UB 用例（`aliasing_ub.rs`）

| 用例 | 形态 |
|---|---|
| `shared_ref_invalidated_by_mut_write` | 从 `&mut` 建 `&T` → 通过 `&mut` 写 → 用 `&T` |
| `shared_write_hidden_behind_black_box` | ★ 跨函数 + `black_box` 藏起铸型点（**rustc 的 lint 抓不到**） |
| `shared_ref_invalidated_by_unsafe_cell_write` | ★ 先建 `&T` 再通过 `UnsafeCell` 写 |
| `dangling_pointer` | `Vec::push` 后使用旧指针 |

★ **用例二和三是本章的重点**：
前者说明"静态检查只能覆盖它看得见的部分"，
后者说明"`UnsafeCell` 不会救活已经存在的 `&T`"。

### ⚠️ 为什么 UB 用例用 `#![cfg(miri)]` 包住整个文件

**普通 `cargo test` 下这些 UB 会"通过"** —— UB 不一定当场崩。
实测：`shared_ref_invalidated_by_mut_write` 在 `-O` 下也照样输出正确数字。
**所以 UB 用例只能给 Miri 跑**，否则 CI 会不稳定。

## ⚠️ Miri 的规则仍是实验性的

每次报 UB，Miri 都会附一句：

```text
= help: this indicates a potential bug in the program: it performed an invalid
        operation, but the Stacked Borrows rules it violated are still experimental
```

**这不是客套话。** 另有一个竞争模型 **Tree Borrows**（`-Zmiri-tree-borrows`）。

| 情形 | 结论 |
|---|---|
| Miri 报 UB | **几乎肯定有问题**，值得认真查 |
| Miri 不报 | **不能**反推"代码一定 sound" |
| 文档里 | **不要**把"Miri 通过"写成 soundness 证明 |

## 交叉验证（可选）

```bash
scripts/verify-miri.sh                                   # 主证据
grep no_retag .evidence/ch25-aliasing-lib.mir            # 澄清用
rustc --edition 2024 --crate-type=lib \
  examples/ch25-aliasing/fail/write_through_shared_ref.rs # rustc 的 lint
```

## 待办

- [x] Miri 用例扩到 4 个测试目标（合法 2 + UB 2），接入 `verify-miri.sh`
- [x] `UnsafeCell` / `PhantomData` 用例落库
- [x] 实测并**修正**了一条常见误解：`UnsafeCell` **不会**救活已存在的 `&T`
- [x] rustc 的 `invalid_reference_casting` lint 已落库为反例并加断言
- [ ] 考虑在"深潜框"里提一句 Tree Borrows（`-Zmiri-tree-borrows`）
- [ ] 第 26 章（何时不该用 unsafe）需要新 example
