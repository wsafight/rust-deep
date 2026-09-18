# 第 11 章（实战）：设计一个小型 trait 抽象层 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch11-project-abstract
scripts/verify-all.sh ch11       # 10 条断言（PASS=12）
```

## 关键结论与断言（10 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | 关联类型版本：`Sum` 单态化 | MIR 里 `<Sum as Projection>::project` |
| 2 | 同一个 trait 的另一个实现 | MIR 里 `<Count as Projection>::project` |
| 3 | GAT 版本：一族 key 类型 | MIR 里 `<KeyedSum as KeyedProjection>::project_keyed` |
| 4 | 生命周期 GAT：MIR 签名里 GAT 参数已消失 | `project_borrowed(_1: &CachedSum, _2: &[u64]) -> &[u64]` |
| 5 | `dyn` 从 vtable 偏移 24 取函数指针 | `sum_dyn` 函数体内 `ldr x?, [x?, #24]` |
| 6 | `dyn` 是间接尾跳转 | `sum_dyn` 函数体内 `br x?` |
| 7 | ★ **去虚化**：调用点已知类型时间接跳转消失 | `use_dyn` 函数体内 `br x?\|blr` **0 次** |
| 8 | 给 trait 加泛型方法 → `dyn` 立刻失效 | `fail/generic_method_kills_dyn.rs` → **E0038** |
| 9 | blanket impl 与具体 impl 冲突 | `fail/conflicting_blanket.rs` → **E0119** |
| 10 | 量化方向反了 | `fail/too_weak_bound.rs` → **E0597** |

## ★ 核心发现：`dyn` 的代价**只在类型真的未知时**才存在

这是本章最有价值的一条实测结果，也是写作前没有预料到的。

`sum_dyn`（参数是 `&dyn Projection`，符号被导出）的完整函数体：

```asm
__RNvCsrtIYgyWToU_3lib7sum_dyn:
	mov	x8, x1
	mov	x1, x0
	ldr	x3, [x3, #24]     ; ← vtable 偏移 24（与第 7 章完全一致）
	mov	x0, x2
	mov	x2, x8
	br	x3                ; ← 间接尾跳转
```

**6 条指令**，其中 `ldr` + `br` 就是动态分发的全部代价。

但同一个 trait，在 `use_dyn` 里传的是**已知的具体类型**：

```rust
#[unsafe(no_mangle)]
pub fn use_dyn(events: &[u64]) -> u64 { sum_dyn(events, &Sum) }
```

实测 `use_dyn` 的函数体里，`br x?` / `blr` 出现 **0 次** ——
LLVM **完全去虚化**了：它知道 `p` 就是 `&Sum`，
于是把 vtable 加载常量折叠掉，直接内联了 `Sum::project`。

> **这是第 7 章"`dyn` 的代价是一次 `ldr` + 一次 `br`"的补完：**
> 那两条指令的代价**只在编译器不知道具体类型时才付**。
> 一旦调用点类型可见（哪怕参数写的是 `&dyn`），整个 vtable 加载就消失了。
>
> 所以"我用了 `dyn` 所以慢"是一个**不成立**的推论 ——
> 要看的是**调用点能不能看见具体类型**。

★ 这条断言必须限定在**函数体内**，不能用全文件 grep：
`.O3.s` 里有几百个符号，`br x?` 到处都是。
为此给 `verify-all.sh` 加了新的断言类型 **`assert_fn_contains` / `assert_fn_not_contains`**
（awk 取 `^符号:` 到 `cfi_endproc` 之间的函数体）。
这是第四种断言类型。

## 五个判据的落点

本章的 API 是"事件存储的投影层"，五个判据各自逼出一个设计决定：

| 判据 | 来自 | 落点 | 实测 |
|---|---|---|---|
| 一个类型能实现几次 | 第 6 章 | `Projection::Out` 用**关联类型** | `<Sum as Projection>::project` |
| 能不能建 vtable | 第 7 章 | `Projection` **可以** `dyn` | `sum_dyn` 有 `ldr #24` + `br` |
| 谁拥有 trait | 第 8 章 | `Counted` 是本地的 → blanket impl 合法 | `impl<T: Projection> Counted for T` 编译通过 |
| 量化方向 | 第 9 章 | `run_pipeline` 需要 `for<'a>` | `fail/too_weak_bound.rs` 报 E0597 |
| 一族类型 | 第 10 章 | `KeyedProjection::Out<K>` 用 **GAT** | `<KeyedSum as KeyedProjection>::project_keyed` |

## ★ 判据之间的**互相牵制**

这是本章要建立的直觉。同一个 API，每加一个需求都会**牺牲另一个能力**：

### 牵制一：加一个泛型方法 → `dyn` 没了

`src/lib.rs` 里的 `Projection` 只有关联类型 `Out`，所以 `dyn Projection` 可用。
`fail/generic_method_kills_dyn.rs` 加了 `fn project_as<T>(&self, ...) -> T`：

```text
error[E0038]: the trait `Projection` is not dyn compatible
  --> examples/ch11-project-abstract/fail/generic_method_kills_dyn.rs:29:20
   |
29 | pub fn use_dyn(p: &dyn Projection<Out = u64>, e: &[u64]) -> u64 {
   |                    ^^^^^^^^^^^^^^^^^^^^^^^^^ `Projection` is not dyn compatible
   |
21 | pub trait Projection {
   |           ---------- this trait is not dyn compatible...
26 |     fn project_as<T>(&self, events: &[u64]) -> T;
   |        ^^^^^^^^^^ ...because method `project_as` has generic type parameters
```

★ **关联类型不破坏 dyn compatibility，泛型方法才破坏。**
这是"关联类型还是泛型参数"这个选择的一个**隐藏权重**——
第 6 章只讲了"一个类型实现几次"，这里补上"会不会毁掉 `dyn`"。

### 牵制二：要"一族 key" → 要么 GAT，要么毁掉 `dyn`

```rust
pub trait KeyedProjection {
    type Out<K>;                        // ← GAT
    fn project_keyed<K: ...>(&self, events: &[(K, u64)]) -> Self::Out<K>;
}
```

GAT **本身**就足以让 trait 失去 dyn compatibility
（第 10 章的 E0038：`because it contains generic associated type Out`）。

所以这个设计里的 `KeyedProjection` **只能是静态分发的**。

### 牵制三：要"借出内部数据" → 必须 GAT + 生命周期参数

```rust
pub trait BorrowingProjection {
    type Out<'a> where Self: 'a;        // ← 生命周期参数的 GAT
    fn project_borrowed<'a>(&'a self, events: &'a [u64]) -> Self::Out<'a>;
}
```

`where Self: 'a` 在这里是**强制**的（第 10 章）。
而参数是**类型**的 GAT（`Out<K>`）**不需要**任何 where 子句 ——
这是本章实测到的另一条容易搞混的规则：

| GAT 参数 | 是否需要 `where Self: 'a` |
|---|---|
| 生命周期 `type Out<'a>` | ✅ 强制 |
| 类型 `type Out<K>` | ❌ 不需要 |

### 牵制四：`for<'a>` vs 签名上的 `'a`

`run_pipeline` 内部要传**自己临时造的切片**：

```rust
pub fn run_pipeline<F>(events: &[u64], f: F) -> u64
where F: for<'a> Fn(&'a [u64]) -> u64      // ← 必须 for<'a>
{
    let doubled: Vec<u64> = events.iter().map(|x| x * 2).collect();
    f(&doubled) + f(events)
}
```

写成 `F: Fn(&'a [u64]) -> u64`（`'a` 在签名上）就报 E0597
（`fail/too_weak_bound.rs`）—— 与第 9 章那条判据完全一致。

## 反例汇总

| 文件 | 错误 | 对应判据 |
|---|---|---|
| `fail/generic_method_kills_dyn.rs` | **E0038** | 第 7 章：泛型方法 → 表长不定 |
| `fail/conflicting_blanket.rs` | **E0119** | 第 8 章：blanket 与具体 impl 冲突 |
| `fail/too_weak_bound.rs` | **E0597** | 第 9 章：量化方向 |

`fail/conflicting_blanket.rs` 的报错：

```text
error[E0119]: conflicting implementations of trait `AllProjections` for type `Sum`
   |
22 | impl<T> AllProjections for T {
   | ---------------------------- first implementation here
   |
28 | impl AllProjections for Sum {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^ conflicting implementation for `Sum`
```

★ 这条反例解释了为什么 stable Rust 里**没有特化**：
想给所有类型一个默认实现，再给某个类型一个特化版本 ——
blanket impl 已经把那个类型覆盖了。

## 交叉验证（可选）

```bash
# dyn 的真实代价（函数体内）
awk '/^__RNvCsrtIYgyWToU_3lib7sum_dyn:/,/cfi_endproc/' .evidence/ch11-project-abstract-lib.O3.s

# 去虚化（0 次间接跳转）
awk '/^_use_dyn:/,/cfi_endproc/' .evidence/ch11-project-abstract-lib.O3.s | grep -cE 'br\s+x[0-9]+|blr'

# 各判据的实例化
grep -nE 'Projection|KeyedProjection' .evidence/ch11-project-abstract-lib.mir
```
