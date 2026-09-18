# Rust 深水区 — 写法与工程方案（临时文档）

> 本文档是写作前的"设计图纸"，放在仓库根目录，**内容确定后删除**。
> 记录：书的核心方法论、证据链、可复现的命令、"深水"边界、章节模板、路线。
> 生成时间基准：**Rust 1.98.1**（`rustc 1.98.1 (48a229cea 2026-09-01)`）/ LLVM 22.1.8 / host `aarch64-apple-darwin`

---

## 0. 一句话方法论

> **每个结论都必须有一条可以亲自跑出来的证据链：源码 → MIR → LLVM IR → 汇编/机器码 → 实测。**

不写"据说是这样"，只写"你可以这样验证：`<命令>`，然后看 `<输出里的哪一行>`"。

---

## 1. "深水"的定义（本书的边界）

不是"讲得深"，而是**每一层都落到可观察的产物上**，并且说清"编译器承诺了什么、没有承诺什么"。

```
你的代码
   │  rustc
   ▼
HIR ──────────── 语法糖展开后（cargo-expand 可看）
   │
   ▼
MIR ──────────── ★ 借用检查器真正工作的 IR；所有权/借用/生命周期在这里被验证
   │              （控制流图 + 每个变量的 StorageLive/StorageDead）
   │              ⚠️ 注意：retag（Stacked Borrows）**不在**这里——
   │                 它在 codegen 阶段，MIR 打印看不到（见 §11.1）
   ▼
LLVM IR ──────── ★ rustc 交给 LLVM 的契约：noalias / readonly / nonnull / dereferenceable / captures
   │              （"Rust 的语义保证"变成"LLVM 可利用的元数据"的地方）
   ▼
汇编 / 机器码 ── ★ 契约在硬件上的兑现：向量化、内联、原子指令、panic 路径
   │
   ▼
运行时实测 ───── criterion 基准 / perf / 长度对比（⚠️ 当前未接入，见 §13 断点 6）
```

**关键洞察（本书的核心论点之一）**：
Rust 的安全保证**不是运行时检查**，而是编译期推导出的、写进 LLVM IR 的**元数据**。
`LLVM` 正是拿这些元数据去做优化。所以：

> **你越能用类型表达"这段内存有什么保证"，LLVM 能做的优化就越多。**
> 反过来，`unsafe` 一旦撒谎（破坏了元数据的前提），就不是"慢一点"，而是**未定义行为**。

这本书就是围绕"契约层"逐层展开的。

---

## 2. 证据链工具与命令（已在本机验证可用）

| 层 | 工具 | 命令 | 状态 |
|---|---|---|---|
| 宏/语法糖 | `cargo-expand` | `cargo expand -p <example>` | ✅ 已装 |
| **MIR** | rustc **stable** | `rustc --edition 2024 --emit=mir -o out.mir x.rs` | ✅ **stable 可用，无需 nightly** |
| LLVM IR | rustc stable | `rustc --emit llvm-ir=out.ll --crate-type=lib x.rs` | ✅ |
| 汇编 | rustc stable | `rustc --emit asm=out.s --crate-type=lib x.rs` | ✅ |
| 反汇编/机器码 | llvm-tools | `llvm-objdump -d --demangle --no-show-raw-insn x.o` | ✅ 组件已装 |
| **跨架构对照** | rustc | `tools/evidence.sh <example> x86_64-apple-darwin` | ✅ 已支持（产物带 `.x86_64` 后缀） |
| 优化对比 | rustc | `-C opt-level=0 / 3`，`-C debuginfo=0` | ✅ |
| 基准 | criterion | `cargo bench` | ⚠️ **未接入**（无 `benches/`、无依赖）—— 见 §13 断点 6 |
| **别名语义（真）** | Miri | `cargo +nightly miri test` | ✅ **已装**（nightly 1.100.0 / miri 0.1.0） |
| 神谕对比 | Godbolt | 只作插图，不作为唯一依据 | — |

> **关于 Miri**：它**不是** rustc 的组件，**stable 不提供**
> （`rustup component list --toolchain stable | grep miri` → 0 条）。
> 它是**独立的 MIR 解释器**，只随 **nightly** 发布。
> 用途：把 MIR 逐条解释执行，同时维护**每个字节的借用栈（Stacked Borrows）**，
> 从而抓到普通运行**完全看不出来**的 UB。
> 代价：慢（解释执行）+ 需要 nightly。
> → 在书里只能作为"深潜框 / 验证工具"，**不进正文的复现路径**。

### 实测发现的坑（必须写进书里的"工具注意事项"）

> 以下全部在 **rustc 1.98.1**（`aarch64-apple-darwin`）上复核过（2026-09-18）。
> 与初版记录不符的，以本节为准。
> 复核过程：先在 1.98.0 上发现初版记录的问题并修正，随后升级到 1.98.1，
> **22 条断言全部复跑通过**，§3 的关键结论（alias 折叠、`#24`、`ldadd`、
> `no_retag`）逐条重验无变化。

1. **★ MIR 用 stable 的 `--emit=mir` 即可，不要装 nightly。**
   - `rustc 1.98.x` stable 上 `--emit=mir` **完全可用**，且支持 `--edition 2024`。
   - 实测确认：**借用检查失败的程序不会生成 MIR**（报 E0502 后 `.mir` 文件不存在），
     说明 `--emit=mir` 输出的是**借用检查之后**的产物 —— 正是本书需要的。
   - **不需要 `-o`**：裸写 `--emit=mir` 就会生成 `x.mir`。只有写 `-o /dev/null`
     才会报 `couldn't create a temp dir: Operation not permitted`（因为 rustc 要在
     `-o` 的**目录**里建临时目录，而 `/dev` 不可写）。给正常路径即可。
   - **不要 `-o <已存在的目录>`**：会报
     `the generated executable ... conflicts with the existing directory`。
   - `--emit=mir` 默认按 **bin** 处理，没有 `fn main` 会报 E0601；
     库代码要加 `--crate-type=lib`（`tools/mir.sh` 已加）。
   - `-Zunpretty=mir` 需 nightly；用 `RUSTC_BOOTSTRAP=1` 走非官方路径时，
     实测输出与 `--emit=mir` **逐字节相同**（可作附录补充，不作正文路径）。
   - `-Zdump-mir`（nightly / bootstrap）能 dump 每个 pass 前后的 MIR，
     **但看不到 retag**——见 §11.1，这是本章最大的坑。
   - 注：`nightly-2024-10-07` 是 1.83，不支持 edition 2024；
     `nightly` 已修好（`rustc 1.100.0-nightly` + `miri 0.1.0`），见 §2 工具表与 §13。
2. **`--emit` 多产物不需要特殊写法，但 `-o` 语义要注意。**
   - 裸写 `--emit asm,obj` 正常工作，生成 `x.s` + `x.o`（**不报错**，初版记录有误）。
   - `--emit asm=x.s,obj=x.o` 同样正常。
   - 只有 `--emit=asm,obj -o out` 会给一条 warning
     （`the explicitly specified output file name will be adapted`），然后生成 `out.s`/`out.o`。
   - 结论：**推荐显式写 `=path`**，理由是可控，不是"否则会报错"。
3. **`#[no_mangle]` 在 edition 2024 下是 unsafe attribute，必须写 `#[unsafe(no_mangle)]`。**
   否则报 E0133 `unsafe attribute used without unsafe`。
   本书所有示例统一用 `#[unsafe(no_mangle)]`；这本身就是 edition 2024 的一课。
4. **macOS AArch64 上不存在"栈金丝雀污染反汇编"的问题。**
   实测：O0 / O3 / `debuginfo=0/2` / 大栈帧 / 带 panic 与 `format!` 的程序，
   `___stack_chk_guard` 出现次数均为 **0**；`-C stack-protector` 这个 codegen
   option 在 1.98 **根本不存在**。→ **初版记录第 4 条作废**，不要写进书里。
5. **`#[no_mangle]` 的泛型函数不行**——泛型必须单态化，名字必然带实例化信息。
6. **看干净汇编的两个实用手段**：
   - 讲"干净指令序列"时用 `#[unsafe(no_mangle)]`，符号名短；
   - 讲"为什么能内联 / 链接"时反而**保留 mangled 名**（它编码了泛型实例化信息，本身就是证据）。
7. **`--emit=asm` 只给 `.s`，`.data` 段里的 vtable 也在里面**——
   实测 `ch07-vtable` 的 `.O3.s` 里能直接读到 `__DATA,__const` 段、
   `.asciz "\000...\b...\b"`（size=8/align=8）与 `.quad ...Shape4area`。
   **不需要 objdump 才能看 vtable 布局**，`--emit=asm` 足够。
   `--emit obj` + `llvm-objdump -s -j __const` 是交叉验证手段。
8. **vtable 只在"真的发生 unsize 强制转换"时才生成。**
   函数签名里有 `&dyn Trait` 参数**不足以**生成 vtable；
   必须有构造点（`Box::new(x)` 返 `Box<dyn T>`、`&x as &dyn T`、`static VT: &dyn T`）。
   见 §3.7 与 `examples/ch07-vtable/src/lib.rs` 的说明。

### 工程脚手架（已落地，2026-09-18）

```
rust-deep/
├── book.toml
├── Cargo.toml                # ★ workspace，members = ["examples/*"]
├── .gitignore                # book/ target/ *.s *.ll *.mir .evidence/
├── PLAN.md                   # 临时设计文档（定稿后删）
├── src/                      # mdBook 正文（只读这里）
├── examples/                 # ★ 证据层：每章一个独立 crate
│   └── chNN-xxx/
│       ├── Cargo.toml
│       ├── src/lib.rs
│       └── evidence.md       # 该章的实测记录（命令 + 真实输出）
├── tools/
│   ├── lib.sh                # 公共函数（版本口径、路径解析、前缀）
│   ├── mir.sh                # MIR（stable --emit=mir，无需 nightly）
│   ├── llvm.sh               # LLVM IR（可指定 opt-level）
│   ├── asm.sh                # 汇编（可指定 opt-level）
│   ├── objdump.sh            # 反汇编（llvm-objdump，优先用 sysroot 里的）
│   └── evidence.sh           # 一次生成某章全部证据（s/ll/mir/o）→ .evidence/
│                             #   第二个参数是 target，可出跨架构对照证据
└── scripts/
    ├── verify-all.sh         # ★ 编译 + 生成证据 + 断言（58 条）
    └── verify-miri.sh        # ★ Miri 验证（3 条，与上面分开，见 §13 断点 3）
```

**每个 example 的目录约定**：
```
examples/chNN-xxx/
├── Cargo.toml
├── src/lib.rs            # 正例（可编译）
├── fail/*.rs             # 反例（故意编译不过，不参与构建）
├── tests/*.rs            # Miri 用例（UB 用例必须用 #![cfg(miri)] 包住）
├── cross-targets         # 可选：声明需要哪些额外 target 的对照证据
└── evidence.md           # 该章的实测记录 + 断言清单
```

**证据产物命名**：`.evidence/<example>-lib.O{0,3}.{s,ll,o}` / `.evidence/<example>-lib.mir`
（由 `RD_PREFIX` 控制前缀，避免各章都叫 `lib.*` 互相覆盖。）
指定 target 时自动加后缀：`.evidence/<example>-lib.O3.x86_64.s`。

**`.evidence/` 与 `book/`、`*.s/*.ll/*.mir/*.o` 全部 gitignore**，靠 `scripts/verify-all.sh` 重新生成。

**为什么值得建 `verify.sh`**：书里宣称的每一个"编译器生成/没生成"的结论，
都应该是**可自动验证的**（例如断言汇编里出现/不出现某条指令）。否则书会腐烂。

**工具接口（写正文时照抄这里，不要凭记忆写）**：
```bash
tools/mir.sh      <example 名> [opt]   # 例：tools/mir.sh ch01-borrow
tools/llvm.sh     <example 名> [opt]
tools/asm.sh      <example 名> [opt]
tools/objdump.sh  <example 名> [opt]   # RD_NO_DUMP=1 只生成 .o
tools/evidence.sh <example 名> [target] # 生成该章全部证据（target 可选）
scripts/verify-all.sh [chNN]           # 编译 + 证据 + 断言（58 条）
scripts/verify-miri.sh                 # Miri 验证（3 条，需 nightly）
```
⚠️ 参数是 **example 名**（`ch01-borrow`），**不是** `.rs` 文件路径。
⚠️ 脚本叫 `scripts/verify-all.sh`，**不叫** `tools/verify.sh`。

---

## 3. 已实测的证据样本（写作时的素材，全部本机真实输出）

> **来源说明**：本节样本**全部已迁进 `examples/`**，可由
> `scripts/verify-all.sh`（58 条断言）与 `scripts/verify-miri.sh`（3 条）复现。
> 各条下面标注了对应的 example 名。**本节现在只是"素材索引"**，
> 真正的证据在 `examples/*/evidence.md`（见 §13 的状态表）。

### 3.1 借用检查的错误信息结构

```rust
let mut v = vec![1, 2, 3];
let first = &v[0];
v.push(4);          // error[E0502]
println!("{first}");
```
输出（真实）：
```
error[E0502]: cannot borrow `v` as mutable because it is also borrowed as immutable
3 |     let first = &v[0];
  |                  - immutable borrow occurs here
4 |     v.push(4);
  |                  ^^^^^^^^^ mutable borrow occurs here
5 |     println!("{first}");
  |                ----- immutable borrow later used here
```
→ 书里的讲法：错误信息的三段标注**就是借用检查器眼中的"三个关键点"**：
借用的产生点、冲突点、**最后一次使用点**。第三点正是 NLL 的核心。

### 3.2 MIR：借用检查器真正看到的世界

用 stable 的 `--emit=mir`（**不是** nightly 的 `-Zunpretty=mir`，见 §2）：
```bash
rustc --edition 2024 --emit=mir -o pt.mir --crate-type=lib pt.rs
```
真实输出（`pt.rs`：`let mut p = Point{x:1.0,y:2.0}; let r = &p; let v = r.x; p.x = 3.0;`）：
```
fn simple() -> f64 {
    let mut _0: f64;
    let mut _1: Point;
    let mut _4: f64;
    scope 1 {
        debug p => _1;
        let _2: &Point;          // ← 借用是"一个具名的临时变量"
        scope 2 {
            debug r => _2;
            let _3: f64;
            scope 3 {
                debug v => _3;
            }
        }
    }

    bb0: {
        _1 = Point { x: const 1f64, y: const 2f64 };
        _2 = &_1;                 // ← 借用在这里"产生"
        _3 = copy ((*_2).0: f64);
        (_1.0: f64) = const 3f64; // ← 写 p.x：在所有借用死亡之后
        _4 = copy (_1.1: f64);
        _0 = Add(copy _3, move _4);
        return;
    }
}
```
**这就是深水区的讲法**：借用检查器不"理解"你的语义，它检查的是一个
**控制流图（CFG）上的数据流事实**：`_2 = &_1` 之后到 `_2` 最后一次被使用之前，
`_1` 不能有冲突访问。NLL = 这个"存活区间"由**使用点**而非**词法块**决定。

> 注意：这个例子只有 **29 行**、单个基本块，是理想的"最小可读样本"。
> 建议把 `pt.rs` 落到 `examples/ch01-borrow`，让第 1 章有一个能直接 `cat` 的 MIR 样本。

### 3.3 LLVM IR：`readonly` 与 `noalias` 的真相（★ 重要，且有一个反直觉结论）

```rust
#[unsafe(no_mangle)] pub fn sum_shared(v: &[u64]) -> u64 { ... }   // &[u64]
#[unsafe(no_mangle)] pub fn sum_mut_ro(v: &mut [u64]) -> u64 { ... } // &mut [u64]，但只读
#[unsafe(no_mangle)] pub fn sum_mut_rw(v: &mut [u64]) -> u64 { ... } // &mut [u64]，且写
```
真实 LLVM IR 签名（rustc 1.98.1，`-O`，实测）：
```llvm
define noundef i64 @sum_mut_ro(ptr noalias nofree noundef nonnull readonly align 8 captures(none) %v.0, ...)
define noundef i64 @sum_mut_rw(ptr noalias nofree noundef nonnull           align 8 captures(none) %v.0, ...)
@sum_shared = unnamed_addr alias i64 (ptr, i64), ptr @sum_mut_ro      ; ★ 见下
```

→ **深水洞察（三层，逐层递进）**：

**(1) `&mut` 带来的不是"readonly 的缺失"，而是 `noalias` + "允许写入"的许可。**
`&mut [u64]` 在只读使用时，LLVM 签名与 `&[u64]` **完全一致**（都是
`noalias + readonly`）。只有真的写了（`sum_mut_rw`），`readonly` 才消失。
所以 `&mut` 的排他性 = **`noalias`**；`&` 的共享性 = `noalias` 但**额外**由
Rust 语义保证"无人写"。**"`&mut` 更慢"是错的**——两者的汇编常常完全相同。

**(2) ★ 更狠的一层：LLVM 直接把两个函数合并成了一个。**
实测 `@sum_shared = unnamed_addr alias i64 (ptr, i64), ptr @sum_mut_ro` ——
LLVM 判定两个函数**逐位等价**，于是把 `sum_shared` 变成一个 alias。
这比"并排贴两段一样的汇编"更有说服力：**不是"看起来一样"，是编译器认定同一个函数。**

**(3) ★ 必须澄清的健全性问题（否则读者会以为 rustc 在撒谎）。**
实测 `two_shared(a: &u64, b: &u64)` 两个参数**都**被标了 `noalias`，
而 `&x, &x` 指向同一块内存在安全 Rust 里完全合法。
为什么这不是 UB？因为 LLVM LangRef 对 `noalias` 的定义是：

> "This guarantee only holds for memory locations that are **modified**,
>  by any means, during the execution of the function."

即 **noalias 只约束"被修改"的内存**。两个只读指针别名同一处 —— 没有任何
内存被修改 —— 不触发 noalias 的任何义务。实测验证（`-O`，运行期断言）：
```rust
let (a, b) = unsafe { (&*p, &*p) };      // 安全 Rust：共享借用同一块内存
read_two(black_box(a), black_box(b))      // 结果 42，断言通过
```
→ 书里必须引这句 LangRef 原文。`&T: noalias` 是**正确**的，不是近似。

**(4) `noalias` 真正的威力在跨参数 / 跨调用时才显现。**
实测 `add_all(dst: &mut [u64], a: &[u64], b: &[u64])`，`-O` 的 AArch64：
```
ldp	q0, q1, [x9, #-32]
ldp	q2, q3, [x9], #64
ldp	q4, q5, [x10, #-32]
ldp	q6, q7, [x10], #64
add.2d	v0, v4, v0        ; ★ 三路独立 SIMD 加法 + 4 路展开
add.2d	v1, v5, v1
add.2d	v2, v6, v2
add.2d	v3, v7, v3
stp	q0, q1, [x11, #-32]
stp	q2, q3, [x11], #64
```
LLVM 敢在这里同时读 `a`、读 `b`、写 `dst` 而不做任何别名检查，**唯一依据就是
三个参数的 `noalias`**。这是 `noalias` 从"元数据"变成"实际性能"的画面。

**(5) 一个容易漏掉的细节：`captures`。**
实测 `&mut [u64]`（写）是 `captures(none)`，而 `&[u64]` 经 `black_box` 传递后
出现过 `captures(address)`。`captures` 描述"callee 能把指针捕获到哪一步"，
是 Rust 特有的、比 `noalias` 更细的约束。第 24 章值得单独讲。

> 这一节的全部命令与输出见 `examples/ch07-vtable` 与本章 §3.5；
> 建议为 §3.3 单开一个 example（`ch24-aliasing`）把 (1)–(5) 全部固化成断言。

### 3.4 边界检查：`assert!` 与 `get_unchecked` 的热路径一样，**失败路径不一样**

```rust
#[unsafe(no_mangle)] pub fn safe(v: &[u8], i: usize) -> u8 { v[i] }
#[unsafe(no_mangle)] pub fn checked_unchecked(v: &[u8], i: usize) -> u8 {
    assert!(i < v.len());
    unsafe { *v.get_unchecked(i) }
}
#[unsafe(no_mangle)] pub fn raw_unchecked(v: &[u8], i: usize) -> u8 { unsafe { *v.get_unchecked(i) } }
```
AArch64 汇编（真实，`-O`）：
```
; safe 与 checked_unchecked 的**热路径逐条相同**：
	cmp	x2, x1
	b.hs	LBB?_2
	ldrb	w0, [x0, x2]
	ret
; raw_unchecked 干脆没有检查：
	ldrb	w0, [x0, x2]
	ret
```
**但失败路径不同**（这才是教学点）：
```
; safe：                bl  ...panicking18panic_bounds_check   （带索引与长度）
; checked_unchecked：   mov w1, #29
;                       bl  ...panicking5panic               （带 "assertion failed: i < v.len()"）
```
→ **深水洞察**：`get_unchecked` 是"**优化提示的语法糖**"，不是"跳过检查"。
若你前面已手动 `assert`，热路径与安全版本**完全一致**——因为 LLVM 会把
assert 的约束传播给 `get_unchecked` 的 UB 假设。
**但两者并不"完全相同"**：失败时的 panic 消息与代码路径不同（一个是边界检查
失败，一个是 assert 失败），文件大小、panic 行为、`#[track_caller]` 的位置都不同。
真正危险的是**删掉 assert 却仍假设成立**——那才是 UB（对比 `raw_unchecked`）。

### 3.5 向量化：安全检查会挡优化，但"模式"也会

`sum(&[u64])`（fold + wrapping_add），`-O` 的 AArch64（真实片段）：
```
LBB0_5:
    ldp q4, q5, [x8, #-32]
    ldp q6, q7, [x8], #64
    add.2d v0, v4, v0
    ...
    addp.2d d0, v0
```
→ LLVM 自动做了 **128-bit 向量化 + 4 路展开**（`add.2d` 是 2×i64 的 SIMD 加法）。

**O0 / O3 的行数对比（实测，务必标注是哪个例子）**：
| 例子 | 源文件 | O0 行数 | O3 行数 |
|---|---|---|---|
| `sum`（`plain.rs` / `tools/asm.sh` 单函数） | `pub fn sum(v:&[u64])->u64` | **333** | 54 |
| `use_borrow`（`examples/ch01-borrow`） | `vec!` + `push` + `len` | **824**（`fold` 出现 **0** 次） | 241 |

> ⚠️ 初版把"333 行"与 ch01 混为一谈：ch01 的例子根本没用 `fold`，
> 它的 O0 是 824 行。**引用数字时必须写明是哪个例子。**

O0 的汇编里能读到**未被内联**的 `Iterator::fold`，符号名形如：
```
__RINvXs2J_..._4core5slice4iter...Iterator4foldyNC..._3p13sum0EB1H_
```
（实测：`grep -c fold` 在 `sum` 的 O0 汇编里命中 9 次。）

→ 书里的讲法：**零成本抽象不是"编译器会原谅你"，而是"当所有信息都可见时，
抽象会被完整擦除"**。把 `-O0` 与 `-O` 并排贴出来，比任何形容词都有说服力。

### 3.6 `Arc::clone` 到指令级：引用计数、`ldadd`、失败路径

```rust
#[unsafe(no_mangle)] pub fn clone_arc(a: &Arc<u64>) -> u64 { let b = Arc::clone(a); *b }
```
AArch64（真实，`-O`，完整函数体）：
```
	ldr	x8, [x0]         ; 取出 ArcInner 指针
	mov	w9, #1
	ldadd	x9, x9, [x8]     ; ★ 原子加 1（fetch-add），返回旧值
	tbnz	x9, #63, LBB1_4  ; 旧值最高位为 1 → 溢出/极端情况
	sub	sp, sp, #48
	stp	x20, x19, [sp, #16]
	stp	x29, x30, [sp, #32]
	add	x29, sp, #32
	str	x8, [sp, #8]
	ldr	x19, [x8, #16]   ; ★ 读 payload（ArcInner 偏移 16）
	mov	x9, #-1
	ldaddl	x9, x8, [x8]     ; ★ 原子减 1 —— drop 路径
	cmp	x8, #1
	b.ne	LBB1_3
	dmb	ishld            ; ★ 内存屏障：drop_slow 前的 acquire 语义
	bl	...Arc<u64>::drop_slow
LBB1_3:
	mov	x0, x19
	ldp	x29, x30, [sp, #32]
	ldp	x20, x19, [sp, #16]
	add	sp, sp, #48
	ret
LBB1_4:
	brk	#0x1             ; ★ 溢出时真的执行 brk 指令
```
→ **深水洞察**：`Arc` 的线程安全**全部**落在这几条指令上。
书里可以直接从这十几条指令讲到：
- 为什么 `Arc<T>: Send` 需要 `T: Send + Sync`（payload 是共享的）；
- 为什么计数器溢出是 `brk #0x1`（实测确实生成，不是文档传说）；
- 为什么 release 路径需要 `dmb ishld`（写入者释放、读者获得）；
- `ArcInner` 的布局（`strong` 在 0，`weak` 在 8，payload 在 **16**）——
  `ldr x19, [x8, #16]` 就是证据。

> 断言已上线：`ldadd`、`dmb`、`brk`（3 条，见 `scripts/verify-all.sh`）。

### 3.7 trait object：间接调用与 vtable 的真实布局

```rust
pub fn dyn_area(s: &dyn Shape) -> f64 { s.area() }
```
AArch64（真实，`-O`）：
```
ldr x1, [x1, #24]   ; ★ 从 vtable 偏移 24 处加载函数指针
br  x1              ; 尾调用（不是 bl，因为没有后续工作）
```
静态版本 `static_area(&Sq)` 被**完全内联**为：
```
ldr  d0, [x0]
fmul d0, d0, d0
```

**vtable 的真实内存布局**（★ 已固化进 `examples/ch07-vtable`，可直接复现）：

前置条件：**必须有 unsize 强制转换的构造点**，vtable 才会生成。
`examples/ch07-vtable/src/lib.rs` 现在有三个：
`make_dyn(v) -> Box<dyn Shape>`、`static STATIC_VT: &(dyn Shape + Sync)`、
以及 `dyn_both(&dyn Shape)`（同时调两个方法）。

`tools/evidence.sh ch07-vtable` 生成的 `.O3.s` 里**直接就能读到 data 段**
（不需要 objdump）：
```asm
	.section	__DATA,__const
	.p2align	3, 0x0
l_anon.684219fb9108ceced0d7d3641a5368d1.0:
	.asciz	"\000\000\000\000\000\000\000\000\b\000\000\000\000\000\000\000\b\000\000\000\000\000\000"
	.quad	__RNvXCsrtIYgyWToU_3libNtB2_2SqNtB2_5Shape4area
	.quad	__RNvXCsrtIYgyWToU_3libNtB2_2SqNtB2_5Shape4name
```
**逐字节读法**：
- 前 24 字节（3 个 slot）= `drop_in_place`（这里是 `\0`×8，即 None/空）、
  `size = 8`、`align = 8`（`.asciz` 里的 `\b` 就是 8）—— 与 `Sq(f64)` 精确吻合；
- 第 4 个 8 字节（偏移 **24**）= 第一个方法 `area`；
- 第 5 个 8 字节（偏移 **32**）= 第二个方法 `name`；
- 且 `.quad` 里出现的顺序**就是 trait 里的声明顺序**。

**与汇编交叉验证**（同一份 `.O3.s`，实测）：
```asm
__RNv...3lib8dyn_area:
	ldr	x1, [x1, #24]     ; ★ 偏移 24 —— 与上面的 slot 位置精确吻合
	br	x1                ; 尾调用（不是 bl，因为没有后续工作）
```
`dyn_both` 里同时出现 `#24`（area）与 `#32`（name）——**两个槽位、按声明顺序**，
这是"vtable 布局"最直接的证据（比只看一个方法强得多）。

→ **这是全书最有说服力的一段**：`dyn` 的抽象代价、`#24` 偏移的来源、
`size_of_val` 为什么能在 `&dyn Trait` 上工作，全部落在这 5 个 slot 上。
**断言已上线**（6 条，见 `scripts/verify-all.sh`）：`#24`、`#32`、`br x?`、
`asciz` 里的 size/align、两个 `.quad ...Shape4area/name`。

### 3.8 泛型单态化：LLVM IR 里的证据

```rust
use std::fmt::Debug;
pub fn dbg_it<T: Debug>(v: &T) -> String { format!("{v:?}") }   // 泛型
pub fn dbg_dyn(v: &dyn Debug) -> String { format!("{v:?}") }    // 动态
#[unsafe(no_mangle)] pub fn call_it() -> String { dbg_it(&1u64) }
#[unsafe(no_mangle)] pub fn call_dyn() -> String { dbg_dyn(&1u64) }
```
`--emit llvm-ir` 真实输出（实测）：
- **泛型版没有自己的 `define`** —— `dbg_it` 被内联进 `call_it`，单态化后的实例
  直接消失在调用者里（`grep '^define'` 只有 6 个 define，没有 `dbg_it`）。
- 而 `dbg_dyn` 有独立的 `define`，里面真实地**构造了一个 vtable 对**：
```llvm
store ptr %v, ptr %args, align 8
store ptr @_RNvXs1g_..._Debug_..._fmt..., ptr %_5.sroa.4...
```
- 以及一条真正的**间接调用**：
```llvm
%1 = getelementptr inbounds nuw i8, ptr %_3.1, i64 24   ; ★ 又是偏移 24！
%2 = load ptr, ptr %1, align 8, !invariant.load !4
%_0 = tail call noundef zeroext i1 %2(...)
```
→ 三条链全部对上：**Rust 源码 → MIR 的 `PointerCoercion(Unsize)` →
LLVM 的 `#24` 偏移**。这就是"深水"该有的样子。
**注意 `!invariant.load`**：这是 `&dyn Debug` 里那个 `&` 带来的元数据
（指向的内容不会变），也是 `noalias` 之外 Rust 特有的另一条契约——第 24 章素材。

---

## 4. 章节模板（每章固定结构）

```markdown
# N. 章节标题

> 一句话：本章要破除的那个误解 / 要建立的直觉。

## N.0 一个会让你卡住的例子
真实场景，能编译或不能编译。先给结论的"错误版本"。

## N.1 表层解释（官方书会怎么讲）
一段，不啰嗦。这是读者已知的部分，快速带过。

## N.2 编译器眼里的样子
### 源码 → MIR
命令 + 关键片段 + 逐行解读（StorageLive / borrow / 存活的临时变量）
### MIR → LLVM IR
命令 + 关键片段 + `noalias/readonly/nonnull` 等元数据说明
### LLVM IR → 汇编
命令 + 关键片段 + 逐条指令解读

## N.3 为什么必须这样设计
回到语义：如果这里不这么约束，会失去什么保证？
（多线程 / 别名 / 生命周期 / 内存安全 角度）

## N.4 反直觉的点（本章的高潮）
至少一条"你以为是 A，其实是 B"，且配实验。

## N.5 亲手验证
```bash
tools/evidence.sh chNN      # 生成该章全部证据
tools/asm.sh      chNN 3    # 单看某一层（参数是 example 名，不是文件路径）
```
明确说明：**看输出的哪一行**算验证成功。
（对应断言在 `scripts/verify-all.sh`，`scripts/verify-all.sh chNN` 只跑本章。）

## N.6 与 unsafe 的关系
本章涉及的保证，一旦用 unsafe 打破，会怎样（UB 的具体形态）。

## N.7 小结
3–5 条可携带的直觉。不重复正文。
```

**硬性约束**：
- 每个断言必须能做成 `scripts/verify-all.sh` 里的自动检查（**不是** `tools/verify.sh`）。
- 汇编片段必须注明：编译器版本、target、opt-level、是否 `#[unsafe(no_mangle)]`。
- 不贴超过 ~30 行的原始输出；超出部分说明"截取哪一段，为何"。
- **凡引用性能/行数数字，必须写明是哪个 example、哪条命令**（见 §3.5 的教训）。

---

## 5. 五部分的"深水锚点"

> 编号说明：本表的序号对应 **§12 目录里的章节号**（vtable = 第 7 章、
> Arc = 第 13 章、unsafe 契约 = 第 24 章）。初版用的 "5.1/5.2/5.7"
> 是旧编号，已废弃，避免与 §9 写作路线对不上。

| 章 | 深水锚点（每部分必须有） | 证据层 |
|---|---|---|
| 1 | 借用检查器 = MIR 上的 CFG 数据流；NLL 的真实含义 | MIR |
| 2 | 生命周期是**约束求解**，不是"时间"；variance 编进子类型关系 | MIR + 类型理论 |
| 7 ★ | vtable 前 3 slot 是 drop/size/align；方法从 `#24` 起、按声明顺序；单态化 vs 间接调用 | 汇编 + `.data` 段 |
| 7 | `dyn` 的 object safety 限制，来源于 vtable 无法表达泛型方法 | 汇编 |
| 12 | `Send/Sync` 是 auto trait，检查发生在**类型层**，零运行时成本 | LLVM IR |
| 13 ★ | `Arc` 的 `ldadd`/`ldaddl`/`dmb`/`brk`；`Mutex` 的平台差异（macOS=pthread，Linux=futex） | 汇编 |
| 16 | 内存序 → AArch64 的 `dmb`/`ldar`/`stlr` 与 x86 的 `mfence`/`lock` | 汇编 |
| 18–19 | `async` 展开成状态机；`Pin` 源于自引用；`Unpin` 是 auto trait | `cargo expand` + MIR |
| 24 ★ | `unsafe` 的唯一义务：**维持 LLVM 元数据的前提**（`noalias`/`captures`/`!invariant.load`） | LLVM IR |

其中 **第 7 章（vtable 布局）**、**第 13 章（Arc 指令）**、**第 24 章（noalias 与 UB）**
是全书最高价值的三段，必须写得最细。三者中 **第 7 章的证据已完全固化并可断言**
（见 §3.7 与 §11.3），第 13 章部分固化，**第 24 章目前只有 PLAN 里的探针，尚无 example**。

---

## 6. 关于"1.98.1"的表述规范（统一口径）

- 正文版本标注：`Rust 1.98.1（2026-09-03 稳定版）` ✅ **已核实存在**
- 构建工具版本：**`rustc 1.98.1` / `cargo 1.98.1` / LLVM 22.1.8**（本机已升级，实测）
- 示例验证目标：`aarch64-apple-darwin`（主力）+ **`x86_64-apple-darwin`**（对照，见下）
- **不要**在书中依赖 nightly-only 的 `-Zunpretty` 作为"读者必须能复现"的路径；
  MIR 章节统一用 **stable 的 `rustc --emit=mir`**（已实测确认可用，见 §2）。

### ⚠️ 版本口径与环境的两个遗留问题（定稿前必须解决）

1. **✅ 版本已对齐**（2026-09-18 升级完成）：
   `static.rust-lang.org` 的 stable channel 是 `1.98.1 (48a229cea 2026-09-01)`，
   channel date `2026-09-03`；本机 `rustup update stable` 后已是 **`rustc 1.98.1`**。
   `.evidence/` 全部产物已用 1.98.1 重新生成，**22 条断言全绿**。
   各章"最后验证"标注、README、preface 已统一为 **1.98.1**。
   ⚠️ 唯一副作用：mangled 符号里的 **crate hash 变了**
   （`Cs9jdNsLYHTiK` → `CsrtIYgyWToU`），已同步更新文档；
   **正文引用汇编时必须说明"符号名里的 hash 每次编译都会变"**，不要当成常量。

2. **对照目标 `x86_64-unknown-linux-gnu` 在本机不可执行。**
   实测 `--target x86_64-unknown-linux-gnu` 直接报 E0463
   （`can't find crate for std`，该 target 未安装），macOS 上也没有 linux 交叉链接器。
   这与 §2"读者零额外安装"的核心决定直接冲突。
   **建议改为 `x86_64-apple-darwin`**：同平台（不引入交叉工具链）、ISA 不同
   （照样能讲 `lock cmpxchg` / SSE 与 NEON 的对照），
   且**只要 `rustup target add x86_64-apple-darwin` 即可**（本机已列在可用 target 中）。
   注意：这样只能**看代码**，不能运行/benchmark。要拿到**可运行**的 x86_64 证据，
   走 CI（GitHub Actions 的 linux runner）。
   **这个决定要早定，它影响所有汇编章节。**

3. **✅ nightly + miri 已修好**（2026-09-18）：
   `rustup toolchain install nightly -c rustc -c miri -c rust-src`
   → 现在是 `rustc 1.100.0-nightly (923c95cdf 2026-09-16)`，
   `cargo +nightly miri --version` = `miri 0.1.0 (923c95cdf5 2026-09-16)`，
   `cargo +nightly miri setup` 已完成（sysroot 在
   `~/Library/Caches/org.rust-lang.miri`）。

   **miri 的定位（见 §13 断点 3）**：
   - **它不是编译器的一部分**，`rustc` 里没有 miri；它是**独立工具**，
     **只有 nightly 提供**（`rustup component list --toolchain stable | grep miri` → 0 条）。
   - 它是 **MIR 解释器**：把编译好的 MIR 逐条**解释执行**，
     同时维护**每个字节的借用栈（Stacked Borrows）**，所以能抓到
     普通运行**完全看不出来**的 UB。
   - **实测**：一段"悬垂指针读取"的代码，`cargo run` 输出正常的数字、
     `-O` 下也照跑不误；`cargo +nightly miri run` 直接报
     `Undefined Behavior: memory access failed: alloc291 has been freed`。
     一段"违反 Stacked Borrows"的代码，`cargo test` **通过**，
     `cargo +nightly miri test` 报
     `trying to retag from <541> for SharedReadOnly permission ... 
      but that tag does not exist in the borrow stack`。
   - **用法**：`cargo +nightly miri test`（推荐，配 `#[test]` 写用例）；
     `cargo +nightly miri run`（bin crate）。
     已在本书 workspace 的 example 上实测可用。
   - ⚠️ **两个代价**：① 慢（解释执行，比原生慢几个数量级）；
     ② **需要 nightly**，而书的其余部分坚持 stable —— 所以 miri 只能作为
     "深潜框 / 验证工具"出现，**不能进正文的复现路径**。
   - ⚠️ 栈金丝雀那条教训的同类：`-Zdump-mir` 看不到 retag（§11.1），
     但 **miri 能**——因为 miri 执行的是 **MIR + 借用栈**，
     而 retag 在 codegen 阶段才插入，两者是不同的东西。
     **第 25 章的证据必须来自 miri，不能来自 MIR 打印。**

---

## 7. 风险与对策

| 风险 | 对策 |
|---|---|
| MIR 输出格式不稳定（rustc 明说不保证） | 只在"深潜框"里用；正文结论不依赖具体输出格式；附工具链版本 |
| **MIR 看不到 retag 全貌**（见 §11.1） | 第 25 章的 MIR 证据改为「Miri 为主 + LLVM IR 为辅」；MIR 只用来讲 no_retag 的**存在** |
| 异步 / `async fn in trait` 快速演进 | 该部分延后写；每章顶部标"最后验证"；预留在线更新 |
| unsafe 章的 soundness 被社区挑错 | 只写能给出完整论证的例子；宁缺勿错；每条 unsafe 必带 safety comment |
| 汇编因平台差异不可复现 | 同时给 AArch64 与 x86_64；用 `verify-all.sh` 对**关键指令**做匹配而非全文对比 |
| 书变得"炫技"、偏离实用性 | §4 模板强制 `N.0` 是真实痛点，`N.7` 必须给可携带直觉 |

---

## 8. 待办 / 待验证

### 已完成（2026-09-18）
- [x] `rustc --emit=mir` 在 stable 上可用（MIR 章节不需 nightly）
- [x] 版本对齐：升级到 **1.98.1**，`.evidence/` 全部重新生成
- [x] 断言：**58 条全绿**（`verify-all.sh`）+ **3 条全绿**（`verify-miri.sh`）
- [x] `tools/objdump.sh` 补 `RD_PREFIX`；`evidence.sh` 生成 `.o`
- [x] `verify-all.sh` 的 filter 作用于断言；新增 `assert_fails`（反例必须编译失败）
- [x] 复核并修正 §2 工具注意事项（`--emit` 多产物、`-o /dev/null`、栈金丝雀、edition 2024）
- [x] 复核并修正 §3.3（补 LangRef 语义 + alias 折叠 + 跨参数向量化）
- [x] 复核并修正 §3.4（热路径一致、**失败路径不同**）
- [x] 复核并修正 §3.6 / §3.8（补 `ArcInner` 偏移、`!invariant.load`）
- [x] 建 9 个 example，把 §3 的探针样本全部迁进仓库（见 §13 状态表）
- [x] nightly + miri 修好；Miri 用例落库（`tests/sb_legal.rs` / `tests/sb_ub.rs`）
- [x] `tools/*.sh` 支持 `RD_TARGET`；`cross-targets` 机制；产出 x86_64 对照证据

### 待办（按优先级）
- [ ] **第 19 章（Pin / Unpin）** —— `ch04-pin` 已有 example，可直接写
- [ ] 第 20–23 章（async 生命周期 / AFIT / tokio / mini runtime）
- [ ] Part V（24–26）正文未写；24/25 的 example 已建
- [ ] 第 26 章（何时不该用 unsafe）需要新 example
- [ ] CI（GitHub Actions linux runner）：拿到**可运行**的 x86_64 证据 + benchmark
- [ ] 接 criterion 做基准（第 5 章 Vec 增长、第 16 章 atomics）；
      **在拿到数据之前正文不写"更快/更慢"**
- [ ] 决定是否加"深潜框"（callout）样式：`> 💡 深潜`，并写进 `book.toml` 的 CSS
- [ ] 把 `evidence.md` 的断言从"脚本硬编码"升级为"文件内标记"
      （§13 断点 2 的 (A) → (B)）

### 正文进度（2026-09-18 更新）

| 章 | 正文 | example | 断言 |
|---|---|---|---|
| 1 借用检查器 | ✅ 414 行 | `ch01-borrow` | 4 |
| 2 生命周期 | ✅ 361 行 | `ch02-lifetimes` | — |
| 3 variance | ✅ 314 行 | `ch03-variance` | — |
| 4 自引用 / Pin | ✅ 358 行 | `ch04-pin` | — |
| 5 实战：树 / 图 | ✅ 273 行 | `ch05-project-tree` | — |
| 6 关联类型 | ✅ 306 行 | `ch06-associated-types` | 7 |
| 7 dyn vs 泛型 | ✅ 347 行 | `ch07-vtable` | 13 |
| **8 coherence** | ✅ **本章完成** | `ch08-coherence` | **12** |
| **9 HRTB** | ✅ **本章完成** | `ch09-hrtb` | 9 |
| **10 GAT** | ✅ **本章完成** | `ch10-gat` | 9 |
| **11 实战：trait 抽象层** | ✅ **本章完成** | `ch11-project-abstract` | 10 |
| **12 Send/Sync** | ✅ **本章完成** | `ch12-send-sync` | 9 |
| **13 Arc/Mutex** | ✅ **本章完成** | `ch13-arc-mutex` | 5 |
| **14 channel** | ✅ **本章完成** | `ch14-channels` | 4 |
| **15 共享可变状态** | ✅ **本章完成** | `ch15-shared-state` | 5 |
| **16 atomics** | ✅ **本章完成** | `ch16-atomics` | 9 |
| **17 实战：线程池** | ✅ **本章完成** | `ch17-project-threadpool` | 10 |
| **18 Future 惰性** | ✅ **本章完成** | `ch18-future` | 11 |
| **19 Pin / Unpin** | ✅ **本章完成** | `ch19-pin` | **13 + 2(Miri)** |
| **20 async 生命周期** | ✅ **本章完成** | `ch20-async-lifetimes` | 19 |
| **21 AFIT** | ✅ **本章完成** | `ch21-afit` | 10 |
| **22 tokio** | ✅ **本章完成** | `ch22-tokio` | 9 |
| **23 实战：mini runtime** | ✅ **本章完成** | `ch23-project-runtime` | 7 + 1(Miri) |
| **24 unsafe 边界** | ✅ **本章完成** | `ch24-noalias` | 11 |
| **25 别名规则** | ✅ **本章完成** | `ch25-aliasing` | 7 + 4(Miri) |
| **26 何时不该用 unsafe** | ✅ **本章完成** | `ch26-when-not-to` | 12 |
| 附录 A/B/C | ✅ **完成** | — | — |

**★ 全书 26 章正文 + 3 个附录全部完成。**

**全量断言：`scripts/verify-all.sh` → 268 PASS / 0 FAIL**
+ `scripts/verify-miri.sh` → 8 PASS / 0 FAIL（2026-09-18 复核）。

### 本批新增的 example（2026-09-18 第二批）

| example | 承载的证据 | 章 | 断言 |
|---|---|---|---|
| `ch09-hrtb` | 省略写法在 MIR 里就是 `for<'a>`（最强证据） | 9 | 9 |
| `ch10-gat` | `Chunks`（借自 self）+ `where Self: 'a` 强制 + GAT 非 dyn | 10 | 9 |
| `ch11-project-abstract` | ★ **去虚化**：`use_dyn` 里间接跳转 0 次 | 11 | 10 |
| `ch15-shared-state` | atomic 4 条 vs Mutex 11+ 条（同一件事） | 15 | 5 |
| `ch17-project-threadpool` | unsize 转换 / 锁作用域 / `Drop` 顺序 | 17 | 10 |

**本批新增的断言类型**：`assert_fn_contains` / `assert_fn_not_contains`
（awk 取函数体，用于"**这个函数里没有**间接跳转"这类断言）。

---

## 9. 写作路线

**第 0 步 ✅ 已完成（见 §13）**：证据链已全部落进 `examples/`，
58 + 3 条断言全绿。**现在可以开始写正文了。**

1. **第 1 章全文**（借用检查器）—— 建立"证据链"的范式，作为全书样板。
   素材已就绪：`examples/ch01-borrow` 有 29 行 MIR 正例 + `fail/E0502.rs` 反例，
   4 条断言全绿。**写完这一章后回头检验范式是否顺手**，再批量推进。
2. 第 2、3 章（生命周期 / variance）—— 完成 Part I 的 MIR 线。
   ⚠️ variance **还没有 example**，需要新开（`-Zdump-variance` 在 1.98 上
   **不存在**，只能靠编译期实验讲）。
3. 第 7 章（vtable，证据已完全固化）+ 第 8 章（coherence）。
   ⚠️ coherence 也还没有 example。
4. 第 12、13、14、16 章 —— **证据已全部固化**，可以直接写。
5. 第 24、25 章 —— 证据已固化（含 Miri），**这两章是"最高价值三段"之二**，
   建议不要留到最后。
6. Part I + II + III 定稿后，再进入异步（IV）。
   ⚠️ Part IV（18–23）目前**一个 example 都没有**，是下一批要建的。

---

## 10. 附：实测命令清单（可直接复制）

```bash
# MIR（stable，无需 nightly）—— 注意 --crate-type=lib，否则会要求 fn main
rustc --edition 2024 --emit=mir -o out.mir --crate-type=lib x.rs

# LLVM IR
rustc --edition 2024 -O --emit llvm-ir=out.ll --crate-type=lib x.rs

# 汇编（vtable 的 data 段也在这里，见 §2 注意事项 7）
rustc --edition 2024 -O --emit asm=out.s --crate-type=lib x.rs

# 反汇编（优先用 rustc 自带的那份，版本才与 rustc 的 LLVM 一致）
"$(dirname $(rustc --print sysroot))/../lib/rustlib/$(rustc -vV | sed -n 's/host: //p')/bin/llvm-objdump" \
  -d --demangle --no-show-raw-insn out.o
# 或直接：tools/objdump.sh <example> 3

# 展开宏与语法糖（注意是 -p，不是 --example）
cargo expand -p ch18-future
```

---

## 11. 补记：2026-09-18 的实测复核（★ 第 25 章必读）

### 11.1 `no_retag` 的真相：stable MIR 里**没有** retag 本体（★ 结论已修正）

初版结论是"retag 在借用检查阶段被折叠，stable 只保留非平凡的 retag 点"。
**这个归因是错的**，2026-09-18 逐 pass 复核后修正如下。

**实测事实**：

| 写法 | `--emit=mir` 里出现什么 |
|---|---|
| `pub fn m(a: &mut u64)`（顶层引用参数） | ❌ 什么都不出现 |
| `pub fn f(a: &mut u64, b: &mut u64)` | ❌ 什么都不出现 |
| `pub fn first<'a>(v: &'a [u64]) -> &'a u64` | ❌ 什么都不出现 |
| `let p = (a,); let q = black_box(p); **q.0` | ✅ `no_retag` |
| `&Box<u64>` 解引用、`&mut &mut u64` 重借用 | ✅ `no_retag` |
| 真实 bin 程序（`vec!` + `format!`） | ✅ `no_retag` |

**机制（读 rustc 源码确认）**：
- 1.98 里 `Rvalue::Use` 带一个 `WithRetag::Yes/No` 枚举
  （`rustc_middle/src/mir/syntax.rs:488,1340`）。
- pretty printer **只在 `No` 时**打印 `no_retag`
  （`rustc_middle/src/mir/pretty.rs:1138`，原文注释：
  *"With retag is more common so we only print when it's without."*）。
- 也就是说：**`--emit=mir` 里能看到的 `no_retag` = "这里故意不做 retag"**，
  而**做了 retag 的地方是静默的**（`WithRetag::Yes` 不打印任何东西）。
- stable 上 `no_retag` 只有两个来源：`EraseDerefTemps`
  （`CopyForDeref` → `Use(.., WithRetag::No)`，源码注释：
  *"We do **NOT** want a retag here!"*）和 `DerefSeparator`。

**更关键的发现**：
- **`-Zdump-mir`（逐 pass dump）里也看不到 retag**——实测对
  `fn f(a: &mut u64, b: &mut u64)` dump 全部 pass，**0 处 retag**。
  1.98 的 pass 列表里**根本没有 AddRetag**；retag 是 **codegen 阶段**的事。
- 证据：`-Zcodegen-emit-retag`（需 bootstrap/nightly）会让汇编里出现
  `bl ___rust_retag_reg`（实测：`&mut` 参数 3 处、嵌套引用 4 处）。
  这才是"retag 真的生成了代码"的证据。

**书中必须这样写**（替换初版措辞）：
> `--emit=mir` 里出现的 `no_retag` **不是** retag 的证据，恰恰相反——
> 它是"这里**不**做 retag"的标记。真正的 retag 在 1.98 里发生在 codegen 阶段，
> MIR 打印（无论 `--emit=mir` 还是 `-Zdump-mir`）都看不到它。
> 要观察 Stacked Borrows 的真实执行语义，用 **Miri**（`cargo +nightly miri test`）；
> 要观察 retag 真的生成了代码，用 **`-Zcodegen-emit-retag`**。

⚠️ **连带后果**：第 25 章原本把"MIR"列为证据层，现在 MIR 只能承担
"展示 `no_retag` 长什么样、纠正误解"的角色，**撑不起"别名规则"的主证据**。
第 25 章的方案需要改为 **Miri 为主 + LLVM IR（`noalias`/`captures`）为辅**。

已写入 `examples/ch25-aliasing/src/lib.rs` 的文件级文档注释中固化，
`verify-all.sh` 的 ch25 断言也改成了准确措辞（`no_retag` 而非 `retag`）。

### 11.2 `--emit` 产物的命名隔离

多个 example 都用 `src/lib.rs`，若输出都叫 `lib.mir` / `lib.O3.s` 会互相覆盖。
已在 `tools/lib.sh` 引入 `RD_PREFIX`，由 `evidence.sh` 设为 example 名，
统一输出为 `.evidence/<example>-lib.O3.s` 形式。
**`tools/objdump.sh` 之前漏了这个前缀**（已修），`evidence.sh` 也补上了 `.o` 生成。

### 11.3 断言已上线（22 条，全绿）

```bash
scripts/verify-all.sh          # 全部
scripts/verify-all.sh ch07     # 只跑某一章（编译、证据、断言都受 filter 约束）
```
当前通过的关键断言：
- `ch01`（4 条）：MIR 里 `_2 = &_1`（借用是具名局部变量）、借用被使用、
  写在借用最后一次使用之后（NLL）；**反例 `fail/E0502.rs` 必须编译失败且报 E0502**
- `ch07`（6 条）：`ldr x?, [x?, #24]` + `ldr x?, [x?, #32]`（两个方法槽位按声明顺序）
  + 间接尾跳转 `br x?` + `.asciz` 里的 size=8/align=8 + 两个 `.quad ...Shape4area/name`
- `ch13`（3 条）：`ldadd`（原子加）+ `dmb`（内存屏障）+ `brk`（溢出路径）
- `ch25`（1 条）：MIR 中 `no_retag`（措辞已按 §11.1 修正）

新增的断言类型 `assert_fails`：**反例必须编译失败**。
这是"证据链"里之前缺的一环——只断言"生成了什么"，没断言"拒绝了什么"。
⚠️ 实现时踩到 macOS bash 3.2 的坑：`$desc（` 里的 `（` 会被当成变量名的一部分，
在 `set -u` 下报 unbound variable。**脚本里所有变量引用必须用 `${var}` 花括号形式。**

---

## 12. 全书目录（已定稿，同步于 `src/SUMMARY.md`）

> 副标题：**Rust 1.98 里那些真正卡人的地方：所有权、Trait、并发、异步与 unsafe**

### 前言

### 第一部分：所有权的心智模型
| # | 章节 | 证据层 | 深水锚点 |
|---|---|---|---|
| 1 | 借用检查器到底在检查什么 | MIR | 借用检查 = CFG 上的数据流，不是"作用域" |
| 2 | 生命周期：标注、省略与推断 | MIR | 生命周期是约束求解，不是"时间" |
| 3 | 协变、逆变与不变 | 类型系统 | variance 编进子类型关系 |
| 4 | 自引用结构与 Pin 的前置知识 | MIR + 布局 | 为什么 self-referential 天然困难 |
| 5 | **实战**：写一个安全的树 / 图容器 | — | 把 1–4 的直觉落到可运行代码 |

### 第二部分：Trait 系统深水区
| # | 章节 | 证据层 | 深水锚点 |
|---|---|---|---|
| 6 | 关联类型 vs 泛型参数 | 类型系统 | 何时用哪个，判据是什么 |
| 7 | dyn vs 泛型：object safety 与单态化代价 | **汇编 + .data 段** | ★ vtable 前 3 slot = drop/size/align，方法从偏移 **24** 起 |
| 8 | coherence、孤儿规则与 blanket impl | 编译期 | 为什么不能给外部类型实现外部 trait |
| 9 | 高阶 trait bound（HRTB） | MIR | `for<'a>` 到底在量化什么 |
| 10 | GAT：泛型关联类型 | 类型系统 | 关联类型带参数后解决什么问题 |
| 11 | **实战**：设计一个小型 trait 抽象层 | — | 综合 6–10 |

### 第三部分：并发
| # | 章节 | 证据层 | 深水锚点 |
|---|---|---|---|
| 12 | Send 与 Sync 的真相 | LLVM IR | 它们是 auto trait，**零运行时成本** |
| 13 | 线程、Arc 与 Mutex | **汇编** | ★ `Arc::clone` 的 `ldadd` / drop 的 `ldaddl` + `dmb`；溢出走 `brk #0x1` |
| 14 | 消息传递：channel 与所有权转移 | 汇编 | 发送即 move，编译期切断别名 |
| 15 | 共享可变状态的所有权设计 | MIR | 用类型表达"谁能改" |
| 16 | 无锁与 atomics 入门 | 汇编 | 内存序如何映射到 `dmb` / `ldaddl` |
| 17 | **实战**：构造一个并发任务池 | — | 综合 12–16 |

### 第四部分：异步
| # | 章节 | 证据层 | 深水锚点 |
|---|---|---|---|
| 18 | Future 是惰性的：手写一个最小 executor | `cargo expand` + MIR | `async` 展开成状态机 |
| 19 | Pin 与 Unpin 为什么存在 | MIR + 布局 | 状态机自引用；`Unpin` 是 auto trait |
| 20 | async 中的生命周期与 Send 传染 | MIR | 跨 `await` 的存活区间 |
| 21 | async fn in trait 的现状 | — | ⚠️ 演进最快的一章，顶部标注"最后验证" |
| 22 | tokio 实战：从原理到工程 | — | 从 executor 原理到生产实践 |
| 23 | **实战**：mini async runtime 或并发 echo server | — | 综合 18–22 |

### 第五部分：unsafe 与 soundness
| # | 章节 | 证据层 | 深水锚点 |
|---|---|---|---|
| 24 | unsafe 的边界哲学：用 unsafe 实现安全接口 | LLVM IR | ★ unsafe 的唯一义务 = 维持 `noalias` 等元数据的前提 |
| 25 | 别名规则、UnsafeCell 与 PhantomData | MIR | ⚠️ retag 有条件可见，见 §11.1 |
| 26 | 何时不该用 unsafe | — | 收尾，给出判断标准 |

### 附录
- A. 环境与版本（Rust 1.98.1 / edition 2024）
- B. 常见编译错误逐条解读
- C. 延伸阅读

### 目录的设计逻辑（不按知识点，按契约层次）

| 部分 | 落到的证据层 | 一句话 |
|---|---|---|
| I 所有权 | MIR | 借用检查是 CFG 数据流；生命周期是约束求解 |
| II Trait | 汇编 + `.data` 段 | vtable 的物理布局决定了抽象代价 |
| III 并发 | 汇编 | Send/Sync 编译期检查；线程安全落在具体原子指令上 |
| IV 异步 | expand + MIR | async 是状态机；Pin 源于自引用 |
| V unsafe | LLVM IR | unsafe 的本质是维护编译器的优化前提 |

**刻意的设计**：
- **每部分末尾都是实战**（5 / 11 / 17 / 23）——读懂与写出来之间隔着一条借用检查器。
- **写作顺序**：I + II + III 先定稿（最扎实、最不过时），再进 IV（演进快）、V（soundness 风险高）。


---

## 13. ★ 证据链状态（2026-09-18 处理完毕）

判定标准：**能否用仓库里的脚本一条命令复现出来**。
`scripts/verify-all.sh` **58 条断言全绿** + `scripts/verify-miri.sh` **3 条全绿**。

### 已建的 example（断点 1 已解决）

| example | 承载的证据 | 章 | 断言数 |
|---|---|---|---|
| `ch01-borrow` | 29 行 `Point` MIR + `fail/E0502.rs` 反例 | 1 | 4 |
| `ch05-bounds` | 热路径 vs 失败路径、检查被消除 | 5 | 4 |
| `ch07-vtable` | vtable 布局（`#24`/`#32`/data 段） | 7 | 13 |
| **`ch08-coherence`** | **孤儿规则四条边界 + fundamental + 单态化证据** | **8** | **12** || `ch12-send-sync` | Send/Sync 是类型层检查 + 2 个反例 | 12 | 3 |
| `ch13-arc-mutex` | Arc 指令 + Mutex 平台差异 | 13 | 5 |
| `ch14-channels` | 发送即 move（MIR 证据） | 14 | 2 |
| `ch16-atomics` | 内存序 → 指令 + **跨架构对照** | 16 | 9 |
| `ch24-noalias` | `noalias`/`readonly`/alias 折叠/LangRef | 24 | 5 |
| `ch25-aliasing` | `no_retag` 澄清 + **Miri 用例** | 25 | 1 + 3(Miri) |

**§3 里原本只活在 `/tmp` 的样本，已全部迁进 example 并加了断言。**

### ch08 的 4 个新反例（2026-09-18）

原 `fail/` 只有 3 个文件，不足以支撑"规则边界"的叙述。新增：

| 文件 | 证明 |
|---|---|
| `fail/orphan_vec.rs` | `Vec<Local>` **不是** fundamental → 仍被 E0117 拒绝（与 `Box<Local>` 逐字对照） |
| `fail/overlapping_blanket.rs` | 两个 blanket impl 在**泛型层面**重叠 → E0119 **不带** `for type` |
| `cross-crate/{upstream,downstream}.rs` + `run.sh` | **跨 crate** 才能证明孤儿规则：下游 blanket impl 上游 trait → E0210 |

★ **`cross-crate/` 的意义**：孤儿规则的动机是"**两个 crate** 不能抢同一个 impl"，
但 `fail/*.rs` 里所有反例都是**单文件 = 单 crate**——
它们在字面上没有复现"跨 crate"这个前提。
`run.sh` 编译两次（先 rlib、再 `--extern`）构造出真正的两个 crate，
这一步补上了动机层面的证据。

为此给 `verify-all.sh` 加了新的断言类型 **`assert_script`**：
脚本退出码必须为 0（脚本内部自行判定"该失败的是否失败了"）。
这是第三种断言类型（前两种是 `assert_contains` 系列和 `assert_fails`）。

★ **`assert_fails` 的坑**：错误码后面紧跟 `]`，
实际输出是 `error[E0119]: conflicting ...`，
所以 pattern 里必须写 `E0119\]: conflicting`，
**不能写 `E0119: conflicting`**（`]` 挡在中间，永远匹配不上）。
写 pattern 时先在 shell 里 `grep` 一遍验证，不要凭直觉拼。

> 排查记录：这个 pattern 失败时一度怀疑是本机 `grep` 被换成了 `ugrep`（确实如此，
> 交互式 shell 里 `grep` 是个 shell function）。但复核后确认 **与 grep 实现无关**——
> `/usr/bin/grep` 对 `E0119: conflicting` 同样返回 0（`]` 挡住了），
> 对 `E0119]:` 返回 1。**结论：断言失败先怀疑 pattern 本身，再怀疑工具。**

### 断点 2（evidence.md 与断言的关系）—— 选了 (A)，保留升级路径

**决定**：先用 **(A) 脚本即真相** —— `evidence.md` 放人读的叙述 + 关键片段，
断言集中在 `verify-all.sh`。
**升级路径**：Part I 定稿时改为 (B)（`evidence.md` 里用 `<!-- assert: ... -->` 标记，
脚本解析它跑），这样"书里的每个断言"与"可执行断言"一一对应。

### 断点 3（第 25 章证据层）—— ✅ 已解决

- Miri 已装好：`rustc 1.100.0-nightly` / `miri 0.1.0`；
- 用例已落库：`examples/ch25-aliasing/tests/sb_legal.rs`（必须通过）+
  `tests/sb_ub.rs`（必须报 UB，用 `#![cfg(miri)]` 包住整个文件）；
- 新增 `scripts/verify-miri.sh`（**与 `verify-all.sh` 分开**，
  因为 UB 用例在普通 `cargo test` 下会"通过"）；
- 第 25 章证据层最终方案：**Miri（主）+ LLVM IR（辅）+ MIR（仅澄清）**。

### 断点 4（对照平台）—— ✅ 部分解决

- `rustup target add x86_64-apple-darwin` 已装；
- `tools/*.sh` 已支持 `RD_TARGET`，产物文件名自动带 `.x86_64` 后缀；
- `examples/*/cross-targets` 声明该 example 需要哪些额外 target 的证据，
  `verify-all.sh` 会自动生成；
- **已产出真实对照证据**（`ch16-atomics`）：AArch64 用 `ldapr`/`ldar`/`stlr`，
  x86_64 上这些**全部退化成 `movq`**，只有 RMW 需要 `lock xaddq`/`lock cmpxchgq`。
- ⚠️ **仍未解决**：x86_64-apple-darwin **只能看代码，不能运行**。
  要拿到**可运行**的 x86_64 证据（含 benchmark），需要 CI（GitHub Actions 的 linux runner）。
  **这是一项待办，不是阻塞项。**

### 断点 5（并发章）—— ✅ 已解决

- `ch12`：Send/Sync 类型层检查 + `Rc`/`Cell` 两个反例（`assert_fails`）；
  ★ 实测踩到"精确捕获让 `unsafe impl Send` 失效"的坑，已写进证据。
- `ch13`：补了 `Mutex`。**实测确认 macOS 上是 `pthread_mutex`，不是 futex**，
  §5 表格的"futex 路径"已修正。
- `ch14`：MIR 里 `send(move _9, move _10)` —— "发送即 move"是字面意义的。
  ★ 实测发现 `mpsc` 内部已经是 **`sync::mpmc`**（不再是老的 Mutex 队列），
  正文要按 `mpmc` 写。
- `ch16`：内存序 → 指令的完整映射 + 跨架构对照。**这一章的对照证据是全书最直观的。**

### 断点 6（基准数据）—— ⚠️ 未解决，但已明确纪律

仍无 `benches/`、无 criterion。**纪律**：
**在拿到真实数据之前，正文里不写任何"更快/更慢"。**
只讲生成的代码（"这条检查被消除了"是事实；"所以快 3 倍"是断言）。

### 断点 7（正文规模）—— 待写

26 章 + 3 附录，只有 `preface.md` 有内容。
**先写第 1 章全文**（素材已就绪：29 行 MIR + 反例 + 4 条断言），
用它验证"证据链范式在写作时真的顺手"，再批量推进。

---
