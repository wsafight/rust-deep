# 附录 C：延伸阅读

> 最后验证：**rustc 1.98.1**（2026-09-01）

本书的目标是"把结论钉在可复现的证据上"。
如果你想**继续往下走** —— 去读产生这些结论的源头 —— 这个附录是路线图。

阅读顺序按"**从证据到源头**"排列，而不是按知名度。

---

## C.1 第一手来源：标准库源码

**本书最重要的延伸阅读不是任何一本书，是标准库的源码。**

它的好处是：**你已经在读了**。`rustc` 安装时会带上 `rust-src`：

```bash
S=$(rustc --print sysroot)/lib/rustlib/src/rust/library
ls $S                      # core / alloc / std / proc_macro ...
```

几个**值得精读**的文件，对应本书的章节：

| 文件 | 看什么 | 章 |
|---|---|---|
| `core/src/pin.rs` | `Pin` 的完整文档 + `get_unchecked_mut` 的 SAFETY 论证 | 4、19 |
| `core/src/cell.rs` | `UnsafeCell` 的全部文档；`Cell` / `RefCell` 怎么建在它上面 | 25 |
| `core/src/marker.rs` | `PhantomData` 的三种用途（生命周期 / auto trait / variance） | 3、12、25 |
| `core/src/future.rs` | `poll` 的签名为什么是 `Pin<&mut Self>` | 18、19 |
| `core/src/ptr/mod.rs` | `addr_of!` / `addr_of_mut!` 与 `&raw const` / `&raw mut` | 19、25 |
| `alloc/src/slice.rs` | `split_at_mut` 的 `unsafe` 实现 —— 边界该怎么收 | 26 |
| `alloc/src/vec/mod.rs` | 最经典的 `unsafe` 边界：`set_len` / `drain` / `IntoIter` | 24、26 |

★ 一个技巧：**读 `SAFETY` 注释**。
标准库里每一处 `unsafe` 都带论证 ——
这是"健全性论证"最好的教材，而且**是真的对的**。

```bash
# 找出标准库里所有的 SAFETY 注释
grep -rn "SAFETY" $S/alloc/src/ | head -40
```

---

## C.2 语言规范与语义

### The Rust Reference

<https://doc.rust-lang.org/reference/>

**规范性的**（不是教程）。本书用到它的地方：

- 生命周期省略规则（第 2 章）；
- dyn compatibility 的定义（第 7 章，注意 1.98 里叫
  "dyn compatible"，旧的 "object safe" 已改名）；
- 孤儿规则与 `#[fundamental]`（第 8 章）；
- HRTB 的语法（第 9 章）。

★ 读法：**当成字典查，不要通读。**

### The Rustonomicon

<https://doc.rust-lang.org/nomicon/>

**"The Dark Arts of Unsafe Rust"** —— 第 24–26 章的官方对应物。

它比本书更"规范"，但**更新较慢**，有些地方已经过时
（比如它讲 retag 的方式与 1.98 的实际实现有出入 —— 见第 25 章）。
读它时保持"回到源码验证"的习惯。

### Unsafe Code Guidelines

<https://github.com/rust-lang/unsafe-code-guidelines>

**别名规则的"立法过程"**。Stacked Borrows 至今仍是实验性的
（第 25 章实测过 Miri 的那句 help）—— 这个仓库就是"它为什么还没定下来"的记录。

★ 值得读 `wip/stacked-borrows.md`：
它是本书第 25 章那个"权限栈"模型的完整定义。

---

## C.3 工具链：本书的证据从哪来

### Miri

<https://github.com/rust-lang/miri>

第 19、25 章的**主证据**来源。README 里有：

- 支持哪些检查（Stacked Borrows / Tree Borrows / 数据竞争 / 泄漏）；
- `-Zmiri-*` 的全部 flag；
- **它查不了什么**（这一点同样重要）。

★ 值得知道的几个 flag：

```bash
cargo +nightly miri test                        # 默认：Stacked Borrows
MIRIFLAGS="-Zmiri-tree-borrows" cargo +nightly miri test   # 换模型
MIRIFLAGS="-Zmiri-backtrace=full" cargo +nightly miri test # 完整回溯
```

### cargo-expand

```bash
cargo install cargo-expand
cargo expand -p ch18-future
```

第 18 章用它看 `async fn` 的展开。**注意是 `-p`，不是 `--example`。**

### rustc dev guide

<https://rust-lang.github.io/rustc-dev-guide/>

想理解本书引用的那些 rustc 内部机制（第 25 章引了
`rustc_middle/src/mir/syntax.rs` 和 `pretty.rs`），从这里开始：

- `mir/` 章节 —— MIR 是什么、有哪些 pass；
- `borrow-check/` 章节 —— 第 1 章那个数据流分析的具体实现；
- `type-inference/` 章节 —— variance 在哪算（第 3 章）。

### LLVM LangRef

<https://llvm.org/docs/LangRef.html>

第 24 章引了 `noalias` 的定义原文。**读它是为了读准**：

> "This guarantee only holds for memory locations that are **modified**,
>  by any means, during the execution of the function."

★ 本书里好几条"反直觉"的结论，都来自**读原文而不是读转述**。

---

## C.4 书

### 《Rust for Rustaceans》（Jon Gjengset）

**与本书重叠最多的一本**。它的深度和本书接近，
但**风格完全不同**：它讲"设计判断"，本书讲"证据链"。

推荐互补着读：

| 主题 | 它的角度 | 本书的角度 |
|---|---|---|
| trait 设计 | 什么时候用哪个 | 编译出什么（第 6、7、11 章） |
| 并发 | 设计模式 | 落到哪条指令（第 13、16 章） |
| unsafe | 抽象的原则 | 元数据的契约（第 24–26 章） |

### 《Programming Rust》（Blandy / Orendorff / Tindall）

比官方书更早、更系统地讲所有权与生命周期。
第 1–3 章的"表层解释"部分，它讲得比本书详细。

### Rust Performance Book

<https://nnethercote.github.io/perf-book/>

第 26 章的延伸。**特别推荐它的"Bounds Checks"一节** ——
本书 26.2 的三组对照，那本书里有更多情形。

★ 它有一句和本书第 26 章完全一致的话：
**先看汇编，再谈优化。**

---

## C.5 视频与课程

### Jon Gjengset 的 *Crust of Rust*

YouTube 上的一系列直播式深潜。与本书高度互补的主题：

- **"Lifetime Annotations"** —— 第 2、9 章的动态讲解；
- **"Subtyping and Variance"** —— 第 3 章（variance 用讲的比用写的清楚）；
- **"The Pin Type"** —— 第 19 章；
- **"Atomics and Memory Ordering"** —— 第 16 章；
- **"Implementing Arc"** —— 第 13 章（从零写一遍 `Arc`）。

★ 他的风格是"边写边讲为什么"，**与本书的"跑出来给你看"正好互补**。

---

## C.6 异步

### The Async Book

<https://rust-lang.github.io/async-book/>

官方异步书。第 18–23 章的背景读物。

⚠️ **它更新较慢** —— `async fn in trait`（第 21 章）在它成书之后才稳定，
所以这一章的内容**不要以它为准**，以本书 + 官方文档为准。

### tokio 文档与教程

<https://tokio.rs/tokio/tutorial>

第 22 章的延伸。特别值得读的是：

- **`tokio::spawn` 的 `Send + 'static` 要求** ——
  第 20 章讲的那个 `!Send` 传染，在真实项目里的形态；
- **`tokio::task::yield_now`** 与取消安全（cancellation safety）——
  本书没覆盖，但是生产代码的必修课。

### `tracing` 与 `futures`

生产异步项目的另外两块：结构化日志（`tracing`）
和 `futures` 的 combinator（`join!` / `select!` / `Stream`）。
本书没有展开 —— 它们是"工程"而非"深水"。

---

## C.7 如果想继续"读源码"

本书引用过的 rustc 内部位置（第 25 章）：

| 位置 | 内容 |
|---|---|
| `compiler/rustc_middle/src/mir/syntax.rs` | `Rvalue::Use` 的 `WithRetag::Yes/No` |
| `compiler/rustc_middle/src/mir/pretty.rs` | 为什么只打印 `no_retag` |
| `compiler/rustc_mir_transform/src/` | `EraseDerefTemps` 等 pass |

拿到源码：

```bash
git clone --depth 1 https://github.com/rust-lang/rust
# 或者只看 MIR 相关的一个 crate
```

★ **一句提醒**：读 rustc 源码的门槛比读标准库高一个数量级。
建议的路径是：

```text
标准库源码（读 SAFETY 注释）
   ↓
rustc dev guide（读概念）
   ↓
rustc 源码（读实现）
```

**不要跳级** —— 直接从 rustc 源码开始，多半会在类型系统里迷路。

---

## C.8 最后：怎么用这些材料

本书每一章的结构是固定的：

```text
章首  先把语法/工具认清            ← 最小语法、直觉锚点与业务场景
N.0 一个会让你卡住的例子
N.1 先把常见说法摆上桌           ← 先建立共同词汇
N.2 编译器眼里的样子             ← 这一节是本书独有的
N.3 为什么必须这样设计
N.4 反直觉的点
N.5 亲手验证
N.6 与 unsafe 的关系
N.7 小结
```

章首和 `N.1` 负责让你先知道“这个工具为什么会出现在项目里”；
本书真正向下扎的部分仍在 `N.2` 和 `N.4`，以及 `N.5` 那些可以亲手复现的命令。

所以：

> 如果你想**理解规则**，读官方书和《Rust for Rustaceans》。
> 如果你想**知道规则为什么是这样**，跑本书的命令，然后读标准库源码。
> 如果你想**参与制定规则**，去
> <https://github.com/rust-lang/unsafe-code-guidelines>。
