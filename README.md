# Rust 深水区

> Rust 1.99 里那些真正卡人的地方：所有权、Trait、并发、异步、unsafe 与资源语义

## 这是什么

写给已经能写 Rust、但仍在借用、trait 组合和异步边界上反复卡住的读者。
30 章主体、一个证据阅读入门章和 4 个附录，从最小反例走向线程池、
mini runtime 与安全 ArrayVec。不是语法大全，也不要求你先会看汇编。

- 从这里开始：[前言与阅读路线](src/preface.md)、[读懂 MIR 与汇编](src/00-reading-evidence.md)。
- 三个样章：[借用检查器](src/part1/01-borrow-checker.md)、[取消安全](src/part6/28-cancellation-safety.md)、[ArrayVec 安全审计](src/part6/30-project-arrayvec.md)。
- 本地阅读：安装 mdBook 后运行 `mdbook serve --open`；验证命令见下文。

本书特点是**可复现的证据链**，但不同证据回答不同问题：

```
源码 → MIR → LLVM IR → 汇编 → 运行时可观察产物
```

汇编只支持当前样本的代码生成结论；Miri 不替代不变式论证，Loom 只检验
构造出的有界模型。详见[证据的边界](src/appendix/d-verification.md)。

## 快速开始

```bash
# 构建书
mdbook serve --open

# 一键验证（格式、Clippy、测试、文档和固定平台证据）
scripts/verify-all.sh

# 跨平台质量检查 / 只验证某一章的证据
scripts/verify-portable.sh
scripts/verify-evidence.sh ch07

# Miri 验证（10 个测试目标 + 前置检查，需要 nightly；与 stable 验证分开）
scripts/verify-miri.sh

# 安装并启用本地 pre-commit 检查
python3 -m pip install pre-commit
python3 -m pre_commit install
python3 -m pre_commit run --all-files
```

## 目录结构

| 路径 | 作用 |
|---|---|
| `src/` | mdBook 正文（唯一的书稿来源） |
| `examples/` | 31 个独立 crate；第 5 章含两个对照样本 |
| `examples/*/src/lib.rs` | 该章的正例（可编译） |
| `examples/*/fail/*.rs` | 该章的**反例**（故意编译不过，不参与构建） |
| `examples/*/tests/*.rs` | 集成测试与 Miri 用例（UB 用例用 `#![cfg(miri)]` 包住） |
| `examples/*/cross-targets` | 该 example 需要哪些额外 target 的对照证据 |
| `examples/*/evidence.md` | 该章的实测记录（命令 + 真实输出 + 断言清单） |
| `tools/` | 生成 MIR / LLVM IR / 汇编 / 反汇编的脚本 |
| `.evidence/` | 证据产物（gitignore，随时可重新生成） |
| `.pre-commit-config.yaml` | 本地提交前检查（格式、Clippy、测试、doctest、证据链） |
| `docs/design-notes-1.98.md` | Rust 1.98 写作阶段的归档设计记录 |

## 证据工具

参数是 **example 名**（`ch07-vtable`），不是 `.rs` 文件路径。

```bash
tools/mir.sh      ch01-borrow        # MIR（stable，无需 nightly）
tools/llvm.sh     ch07-vtable 3      # LLVM IR，opt-level=3
tools/asm.sh      ch07-vtable 0      # 汇编，opt-level=0
tools/objdump.sh  ch13-arc-mutex 3   # 反汇编
tools/evidence.sh ch07-vtable        # 一次生成该章的 MIR / LLVM IR / 汇编

# 跨架构对照（产物带 .x86_64 后缀，只出代码，不能运行）
tools/evidence.sh ch16-atomics x86_64-apple-darwin
```

## 版本口径

- 目标版本：**Rust 1.99.0**（stable），Edition 2024
- **本机工具链：`rustc 1.99.0` / `cargo 1.99.0` / LLVM 23.1.1**
  → `.evidence/` 全部产物与各章"最后验证"标注均为 **1.99.0**。
- 主验证目标：`aarch64-apple-darwin`
- 对照目标：`x86_64-apple-darwin`（可选；**只生成代码，不运行**）
- Miri：固定 `nightly-2026-09-16`；安装方式见附录 A
- 提交前检查：安装并运行 `.pre-commit-config.yaml` 中的本地 hooks（见附录 A）

## 当前状态

- `scripts/verify-portable.sh` 是跨平台门禁；`scripts/verify-evidence.sh` 固定验证
  Rust 1.99.0 / AArch64 macOS 证据。未安装的可选 x86_64 对照目标会标为 `SKIP`。
- GitHub Actions 在 pull request 上运行跨平台检查、证据链和 Miri；三者通过后
  `main` 分支才会部署到 Pages。
- 第 16 章 Loom 正反模型进入跨平台测试；第 30 章 panic guard、Clone 与
  iterator 测试进入 Miri，并有“先发布长度”的预期 UB 探针。
- 跨平台质量检查在 Ubuntu 与 macOS 上运行，包含 Stream 背压测试与原生
  C ABI 集成探针；固定平台汇编断言仍单独运行。
- 本轮改造范围与取舍见 [改造记录](docs/improvements.md)。
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
  | `ch22-tokio` | `Send + 'static` 的两半（依赖 tokio） | 22 |
  | `ch23-project-runtime` | mini executor + 手写 waker（**Miri 用例**） | 23 |
  | `ch24-noalias` | `noalias`/`readonly`/alias 折叠 | 24 |
  | `ch25-aliasing` | `UnsafeCell`/`PhantomData` + **Miri 用例** | 25 |
  | `ch26-when-not-to` | 安全版本 vs `unsafe` 版本的指令数对照 | 26 |
  | `ch27-drop` | drop flags / cleanup / drop check | 27 |
  | `ch28-cancellation` | abort / `select!` / timeout 的取消语义（依赖 tokio） | 28 |
  | `ch29-layout-validity` | 布局 / niche / `MaybeUninit` / 有效值 | 29 |
  | `ch30-arrayvec` | 初始化前缀 / 移动 / 析构责任（**Miri 用例**） | 30 |

- 正文范围：**第 0 章入门 + 30 章主体 + 4 个附录**；持续迭代阅读体验与验证覆盖。
