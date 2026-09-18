# 附录 A：环境与版本

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## A.1 版本口径

| 项 | 值 |
|---|---|
| Rust | **1.98.1**（stable，2026-09-03 发布） |
| Edition | **2024** |
| LLVM | **22.1.8** |
| 主验证目标 | `aarch64-apple-darwin` |
| 对照目标 | `x86_64-apple-darwin`（**只能看代码，不能运行**） |
| Miri | `miri 0.1.0 (923c95cdf5 2026-09-16)`（`rustc 1.100.0-nightly`） |
| tokio（仅第 22 章） | `1.53.1`（唯一的第三方依赖） |

核对本机版本：

```bash
rustc -vV
# rustc 1.98.1 (48a229cea 2026-09-01)
# host: aarch64-apple-darwin
# LLVM version: 22.1.8
```

> ⚠️ **符号名里的 hash 会变。** 汇编里形如
> `__RNvCsrtIYgyWToU_3lib7sum_dyn` 的那段 `CsrtIYgyWToU` 是 crate hash，
> **每次编译都可能不同**（换工具链、改代码、改路径都会变）。
> 引用汇编时不要把它当常量 —— 本书的断言一律只匹配指令与偏移。

## A.2 安装（零额外安装的路径）

正文里所有命令**只需要 stable**：

```bash
# 只需要这个（如果还没有）
rustup toolchain install stable
rustup update stable        # 确保是 1.98.1
```

### Miri（可选，用于第 19、23、25 章）

Miri **不在 stable 里** —— 它是独立的 MIR 解释器，只随 nightly 发布。

```bash
rustup toolchain install nightly -c rustc -c miri -c rust-src
cargo +nightly miri setup
```

验证：

```bash
cargo +nightly miri --version
# miri 0.1.0 (923c95cdf5 2026-09-16)
```

⚠️ 两个代价：

1. **慢** —— 解释执行，比原生慢几个数量级；
2. **需要 nightly** —— 而书的其余部分坚持 stable。

所以 Miri 只作为**深潜框 / 验证工具**出现，**不进正文的复现路径**。

### 对照目标（可选，用于跨架构对照）

```bash
rustup target add x86_64-apple-darwin
```

⚠️ 这只能**看代码**，不能运行 —— 但足以讲清
`lock cmpxchg`（x86）与 `ldaddal`（AArch64）的对照（第 16 章）。

## A.3 复现全书证据

```bash
# 构建书
mdbook serve --open

# 一键验证全书证据（编译 + 生成证据 + 断言，268 条）
scripts/verify-all.sh

# 只验证某一章（编译、证据、断言都受此过滤）
scripts/verify-all.sh ch07

# Miri 验证（8 条，需要 nightly；与上面分开）
scripts/verify-miri.sh
```

### 证据工具

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

### 底层命令（工具脚本做的事）

```bash
# MIR（stable）
rustc --edition 2024 --emit=mir -o out.mir --crate-type=lib x.rs

# LLVM IR
rustc --edition 2024 -O --emit llvm-ir=out.ll --crate-type=lib x.rs

# 汇编
rustc --edition 2024 -O --emit asm=out.s --crate-type=lib x.rs

# 展开宏与语法糖（注意是 -p，不是 --example）
cargo expand -p ch18-future

# ⚠️ 若该 example 同时有 lib.rs 和 main.rs，必须指定目标
cargo expand -p ch22-tokio --bin ch22-tokio
```

### ⚠️ 三个容易踩的坑（都实测过）

1. **库代码要加 `--crate-type=lib`**，
   否则 rustc 会按 bin 处理，报"找不到 `main` 函数"；
2. **`-o` 后面不能是已存在的目录**，会报
   `conflicts with the existing directory`；
   另外 `-o /dev/null` 会报 `couldn't create a temp dir`（`/dev` 不可写）；
3. **`-Zunpretty=mir` 需要 nightly。**
   本书一律用 stable 的 `--emit=mir`，读者零额外安装。

## A.4 平台差异清单

本书的主验证目标是 macOS / AArch64，但**书里的结论都标注了平台**。
遇到差异的地方：

| 主题 | AArch64 | x86_64 | 章 |
|---|---|---|---|
| 原子加 | `ldadd` / `ldaddl` | `lock xadd` | 13、16 |
| 内存屏障 | `dmb` / `ldar` / `stlr` | `mfence` / `lock` 前缀 | 16 |
| `Mutex` 实现 | **pthread**（macOS） | futex（Linux） | 13 |
| 溢出陷阱 | `brk #0x1` | `ud2` | 13 |

★ 结论层面**没有平台差异**的部分（借用检查、生命周期、trait 解析、
`Send`/`Sync`、`Pin`、别名规则）—— 那些是**语言语义**，与目标无关。

## A.5 目录结构

| 路径 | 作用 |
|---|---|
| `src/` | mdBook 正文（唯一的书稿来源） |
| `examples/` | 每章一个独立 crate，承载书中所有可运行示例 |
| `examples/*/src/lib.rs` | 该章的正例（可编译） |
| `examples/*/fail/*.rs` | 该章的**反例**（故意编译不过，不参与构建） |
| `examples/*/tests/*.rs` | Miri 用例（UB 用例用 `#![cfg(miri)]` 包住） |
| `examples/*/cross-targets` | 该 example 需要哪些额外 target 的对照证据 |
| `examples/*/evidence.md` | 该章的实测记录（命令 + 真实输出 + 断言清单） |
| `examples/*/externs` | 该 example 需要哪些第三方 crate（只有 `ch22-tokio` 有） |
| `tools/` | 生成 MIR / LLVM IR / 汇编 / 反汇编的脚本 |
| `.evidence/` | 证据产物（gitignore，随时可重新生成） |

### ⚠️ 唯一依赖第三方 crate 的一章

第 22 章（tokio）需要真的链接 tokio。机制是：

1. `examples/ch22-tokio/externs` 里逐行写需要哪些 crate；
2. `tools/lib.sh` 的 `rd_extern_args` 用 `cargo build --message-format=json`
   找出 `.rlib` 路径，拼出 `--extern=tokio=...` **以及 `-L dependency=<deps 目录>`**；
3. `tools/*.sh` 和 `scripts/verify-all.sh` 都调用它。

★ **`-L dependency=` 是必需的**：只给 `--extern` 不够 ——
rustc 会从 tokio 的 metadata 里读到它依赖 `pin_project_lite` 等 crate，
然后按 `-L` 的搜索路径去找。缺了会报：

```text
error[E0463]: can't find crate for `pin_project_lite` which `tokio` depends on
```

（cargo 自动做这件事，裸 `rustc` 不会。）
