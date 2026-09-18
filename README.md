# Rust 深水区

> Rust 1.98 里那些真正卡人的地方：所有权、Trait、并发、异步与 unsafe

## 这是什么

一本 mdBook 写的书。与"再讲一遍官方书"不同，本书的每个结论都落到
**可复现的证据链**：

```
源码 → MIR → LLVM IR → 汇编 → 运行时可观察产物
```

## 快速开始

```bash
# 构建书
mdbook serve --open

# 一键验证全书证据（编译 + 生成证据 + 断言，268 条）
scripts/verify-all.sh

# 只验证某一章（编译、证据、断言都受此过滤）
scripts/verify-all.sh ch07

# Miri 验证（8 条，需要 nightly；与上面分开，见 PLAN §13 断点 3）
scripts/verify-miri.sh
```

## 目录结构

| 路径 | 作用 |
|---|---|
| `src/` | mdBook 正文（唯一的书稿来源） |
| `examples/` | 每章一个独立 crate，承载书中所有可运行示例 |
| `examples/*/src/lib.rs` | 该章的正例（可编译） |
| `examples/*/fail/*.rs` | 该章的**反例**（故意编译不过，不参与构建） |
| `examples/*/tests/*.rs` | Miri 用例（UB 用例用 `#![cfg(miri)]` 包住） |
| `examples/*/cross-targets` | 该 example 需要哪些额外 target 的对照证据 |
| `examples/*/evidence.md` | 该章的实测记录（命令 + 真实输出 + 断言清单） |
| `tools/` | 生成 MIR / LLVM IR / 汇编 / 反汇编的脚本 |
| `.evidence/` | 证据产物（gitignore，随时可重新生成） |
| `PLAN.md` | 写作方案与实测笔记（临时，定稿后删除） |

## 证据工具

参数是 **example 名**（`ch07-vtable`），不是 `.rs` 文件路径。

```bash
tools/mir.sh      ch01-borrow        # MIR（stable，无需 nightly）
tools/llvm.sh     ch07-vtable 3      # LLVM IR，opt-level=3
tools/asm.sh      ch07-vtable 0      # 汇编，opt-level=0
tools/objdump.sh  ch13-arc-mutex 3   # 反汇编
tools/evidence.sh ch07-vtable        # 一次生成该章全部证据（s/ll/mir/o）

# 跨架构对照（产物带 .x86_64 后缀，只出代码，不能运行）
tools/evidence.sh ch16-atomics x86_64-apple-darwin
```

## 版本口径

- 目标版本：**Rust 1.98.1**（2026-09-03 稳定版），Edition 2024
- **本机工具链：`rustc 1.98.1` / `cargo 1.98.1` / LLVM 22.1.8**
  → `.evidence/` 全部产物与各章"最后验证"标注均为 **1.98.1**。
- 主验证目标：`aarch64-apple-darwin`
- 对照目标：`x86_64-apple-darwin`（已装；**只能看代码，不能运行**）
- Miri：`cargo +nightly miri test`（`rustc 1.100.0-nightly` / `miri 0.1.0`）

## 当前状态

- 断言：**268 条全绿**（`scripts/verify-all.sh`）+ **8 条全绿**（`scripts/verify-miri.sh`）
- 已固化证据的 example（含断言）：

  | example | 内容 | 章 |
  |---|---|---|
  | `ch01-borrow` | 29 行 MIR 样本 + E0502 反例 | 1 |
  | `ch05-bounds` | 边界检查的热路径 vs 失败路径 | 5 |
  | `ch07-vtable` | vtable 布局（`#24`/`#32`/data 段） | 7 |
  | `ch08-coherence` | 孤儿规则四条边界 + 跨 crate 探针 | 8 |
  | `ch09-hrtb` | 省略写法在 MIR 里就是 `for<'a>` | 9 |
  | `ch10-gat` | 借自 self 的迭代器 + `where Self: 'a` | 10 |
  | `ch11-project-abstract` | 去虚化（`use_dyn` 里 0 次间接跳转） | 11 |
  | `ch12-send-sync` | Send/Sync 是类型层检查 + 反例 | 12 |
  | `ch13-arc-mutex` | Arc 指令 + Mutex 平台差异 | 13 |
  | `ch14-channels` | 发送即 move（MIR 证据） | 14 |
  | `ch15-shared-state` | 四种设计的代价对照 | 15 |
  | `ch16-atomics` | 内存序 → 指令 + **跨架构对照** | 16 |
  | `ch17-project-threadpool` | unsize / 锁作用域 / Drop 顺序 | 17 |
  | `ch18-future` | 状态机布局 + 惰性的运行期证据 | 18 |
  | `ch19-pin` | 自引用（一条 `stp`）+ `!Unpin` 的保守性 | 19 |
  | `ch20-async-lifetimes` | 跨 `await` 的借用字段 + `Send` 传染 | 20 |
  | `ch21-afit` | AFIT 单态化（零分配）+ `dyn`/`Send` 的冲突 | 21 |
  | `ch22-tokio` | `Send + 'static` 的两半（★ 唯一依赖 tokio） | 22 |
  | `ch23-project-runtime` | mini executor + 手写 waker（**Miri 用例**） | 23 |
  | `ch24-noalias` | `noalias`/`readonly`/alias 折叠 | 24 |
  | `ch25-aliasing` | `UnsafeCell`/`PhantomData` + **Miri 用例** | 25 |
  | `ch26-when-not-to` | 安全版本 vs `unsafe` 版本的指令数对照 | 26 |

- 正文进度：**全书 26 章 + 3 个附录已完成**
