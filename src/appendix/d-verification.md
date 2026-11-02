# 附录 D：Rust 正确性验证工具箱

> 一句话：没有一种工具能证明 Rust 程序“完全正确”；有效的方法是先写清楚主张，
> 再选择能反驳这类主张的工具。

## D.0 证据的边界：先标注主张的性质

| 主张性质 | 可依赖的边界 | 本书例子 |
|---|---|---|
| 语言 / 标准库契约 | 由 Reference 或 API 文档规定，注意其前提 | `mem::forget` 是安全函数；`Send` 的契约 |
| 当前编译器实现 | 只约束已记录的 rustc 与编译选项 | MIR 形状、诊断文本、vtable 偏移 |
| 目标平台观察 | 同时依赖架构、操作系统或 ABI | 原子指令、Mutex 后端、参数寄存器 |
| 路径级动态检测 | 只检查执行的输入与分支及工具模型 | Miri、sanitizer、fuzz 的当前用例 |
| 有界模型探索 | 覆盖所构造模型中工具探索的执行，不覆盖任意程序 | 第 16 章 Loom 两线程单次发布 |

这不是从弱到强的单一排名：汇编回答指令映射，Miri 反驳某些 unsafe 路径，
Loom 构造同步协议的反例。不能拿一种证据替代另一种，也不能把 Miri 与 Loom
都概括为“跑过一次所以正确”。

阅读结论时，先问“这是 API 承诺，还是这次实验的结果”。如果缺少前提，
回到该章 evidence.md 查工具链、目标和样本；仍不明确，就不把它用于 API 设计。

## D.1 先问：你想证明什么

“测试通过”不是一个足够精确的结论。下面这些主张需要完全不同的证据：

| 主张 | 首选证据 |
|---|---|
| 这个输入得到这个输出 | 单元测试 / 集成测试 |
| 对大量随机输入都满足某个性质 | property testing / fuzzing |
| unsafe 路径没有越界、use-after-free、非法值 | Miri / sanitizer + 不变式论证 |
| 所有关注的线程交错都不死锁、不丢更新 | Loom 模型检查 |
| 优化后没有边界检查或堆分配 | LLVM IR / 汇编 / allocator 观测 |
| 某个反例必须被类型系统拒绝 | compile-fail / UI test |
| C ABI 与头文件约定一致 | `repr(C)` 布局断言 + 跨语言集成测试 |

工具只回答它被问到的问题。汇编里没有 `panic_bounds_check`，不能证明算法不会写错
别的地址；Miri 跑过一个测试，也不能证明未覆盖输入和分支都 sound。

## D.2 本书已经使用的四层证据

### 编译成功与 compile-fail

适合验证类型系统边界：生命周期、coherence、`Send`、dyn compatibility、packed
字段引用等。本书的 `scripts/verify-evidence.sh` 会逐个编译 `fail/*.rs`，并断言
错误码或关键诊断文本。

它能证明“当前 rustc 拒绝这个具体程序”，不能证明所有同类错误都会被拒绝。
诊断文本也不是稳定语言 API，所以断言应优先使用错误码和最小关键词。

### MIR

适合观察 move、drop、状态机字段、unwind edge 和单态化后的调用。第 27、28、30
章分别用它查看 drop flag、取消时被保存的字段、以及已初始化前缀的操作。

MIR 输出面向人类调试，不是稳定格式；它也不是 borrow checker 内部事实的完整
dump，更不是运行期别名模型本身。第 25 章的 `no_retag` 就展示了“看见一个词”
不等于机制发生在这一层。

### LLVM IR 与汇编

适合验证编译器向后端提供的属性，以及这些属性是否落实为指令。本书用它观察
`noalias`、向量化、原子内存序、间接跳转和堆分配调用。

它只能证明特定 rustc、target、优化级别与样本。第 30 章中裸数组被向量化而
`ArrayVec` 没有，正说明“零成本抽象”必须逐个样本验证，不能当作口号。

### 运行期测试

适合业务行为、析构顺序、任务取消和端到端协议。优点是结论接近用户真正观察到
的行为；缺点是只覆盖实际执行的输入、分支、调度与平台。

## D.3 Miri：解释执行 unsafe 路径

Miri 在 MIR 上解释执行，并检查普通硬件通常不会立即暴露的问题，例如：

- 越界、use-after-free、double free；
- 读取未初始化内存；
- 构造非法 `bool`、引用或 enum 判别值；
- 违反当前别名模型的访问；
- 一部分数据竞争和线程 API 错误。

本仓库将它与 stable 门禁分开：

```bash
scripts/verify-miri.sh
```

脚本同时包含“必须通过”和“必须报 UB”的目标。后者很重要：只跑合法代码只能
证明工具没有误报，不能证明它真的覆盖了你关心的错误类别。

Miri 的边界：

- 只执行测试实际走到的路径；
- 很慢，不适合大规模负载；
- 很多 FFI、系统调用和平台行为无法直接解释；
- Stacked Borrows / Tree Borrows 等模型仍在演进；
- **Miri 通过不等于 soundness 证明**。

对 unsafe 抽象，正确顺序是先写不变式和每个 unsafe block 的证明，再让 Miri 尝试
反驳测试覆盖到的路径。

## D.4 Loom：枚举并发交错

普通并发测试即使循环一万次，也可能永远碰不到那个关键调度。Loom 用自己的
atomic、Mutex、Arc 和线程模型替换标准库同步原语，在受控状态空间内枚举可能的
执行交错。

典型结构是把同步原语放在一个薄适配层后：

```rust,ignore
#[cfg(not(loom))]
use std::sync::{Arc, Mutex};

#[cfg(loom)]
use loom::sync::{Arc, Mutex};
```

测试再放进 `loom::model(|| { ... })`。适合检查：

- 某个 Relaxed/Acquire/Release 协议是否真的建立 happens-before；
- 两个线程的特定交错是否丢更新；
- 自定义同步器是否可能死锁；
- 引用计数或状态机转换是否遗漏一个分支。

Loom 不是“给现有测试加一个 flag”。业务代码必须通过 Loom 提供的模型原语运行，
线程数、循环次数和状态数也要刻意压小，否则状态空间会爆炸。

它证明的是模型边界内的所有交错，不是无限线程、无限循环、真实内核和全部 CPU
实现。模型与生产代码的适配层是否一致，仍需代码审查与其他测试保证。

## D.5 Sanitizer：在真实机器上观察错误

编译器 sanitizer 给生成代码插桩，在真实执行中检测特定错误类别：

| 工具 | 主要目标 |
|---|---|
| AddressSanitizer | 越界、use-after-free 等地址错误 |
| ThreadSanitizer | 数据竞争 |
| MemorySanitizer | 未初始化读取（平台支持受限） |
| LeakSanitizer | 泄漏 |

Rust 通常通过 nightly 的 `-Zsanitizer=...` 启用，支持情况与目标平台相关。示意命令：

```bash
RUSTFLAGS="-Zsanitizer=address" cargo +nightly test -Zbuild-std
RUSTFLAGS="-Zsanitizer=thread" cargo +nightly test -Zbuild-std
```

具体 flags 应以所用 nightly 和目标的 rustc 文档为准，不应把这里的示意命令直接
固化成跨平台 CI。

与 Miri 相比，sanitizer 能运行更真实、更大的工作负载，也更适合包含 C/C++ 的
程序；但它同样只检查实际执行路径，插桩也不覆盖所有 UB 类别。ThreadSanitizer
寻找发生过的数据竞争，Loom 则系统枚举一个缩小模型的调度，两者互补。

## D.6 Fuzzing：让输入空间主动攻击解析器

Fuzzing 适合字节解析、状态机、协议、反序列化和复杂边界条件。一个有效 fuzz
target 通常包含明确的 oracle：

- 不应 panic 或越界；
- 编码再解码得到原值；
- 新旧实现结果一致；
- 两个独立解析器对同一输入达成一致；
- 成功解析后的对象满足结构不变式。

只有“调用一下函数，没崩就算过”的 target 价值有限。对于安全 Rust，panic 仍可
能代表拒绝服务或逻辑错误；对于 unsafe 代码，最好把小输入 fuzz 与 sanitizer、
Miri corpus 回放组合起来。

Fuzzer 发现的是一个反例，不是正确性证明。保留最小化后的 crash input，转成普通
回归测试，才能防止同一问题复发。

## D.7 Property testing：从样例提升到性质

单元测试会写：`sort([3, 1, 2]) == [1, 2, 3]`。性质测试更适合表达：

```text
sort(xs) 保持长度
sort(xs) 的输出单调不降
sort(xs) 与 xs 含相同多重集合
sort(sort(xs)) == sort(xs)
```

`proptest`、QuickCheck 一类工具会生成输入并缩小失败案例。它们仍然是抽样，但
比人工列几个边界值更擅长覆盖组合空间。

对第 30 章的容器，可以生成 push/pop/insert/remove 操作序列，同时维护一个
标准 `Vec` 作为 reference model，每一步比较内容与返回值。这类 state-machine
property test 比单独测试每个方法更容易发现交互错误。

## D.8 一套实际可执行的验证顺序

对普通安全 Rust：

1. 单元与集成测试覆盖业务行为；
2. compile-fail 固化重要类型边界；
3. property test 或 fuzz 覆盖输入组合；
4. 只在有性能主张时检查 IR、汇编和 benchmark。

对并发抽象：

1. 写出状态机和内存序论证；
2. 用 Loom 检查缩小模型的交错；
3. 用真实线程做压力测试；
4. 条件允许时增加 ThreadSanitizer。

对 unsafe 抽象：

1. 写出表示不变式和每个 unsafe block 的前置条件；
2. 正例、边界例和 compile-fail API 约束；
3. Miri 跑合法路径与故意 UB 的负向探针；
4. fuzz / property test 生成操作序列；
5. FFI 或大负载再加 sanitizer；
6. 最后检查优化主张，不用性能证据替代 soundness 证据。

## D.9 如何解读“全绿”

全绿只意味着仓库里写下的这些断言，在固定工具链和已执行路径上成立。它不意味：

- Rust 编译器证明了业务逻辑；
- Miri 穷举了所有输入；
- Loom 模拟了完整生产环境；
- 汇编短就一定更快；
- 没有 sanitizer 报告就没有 UB。

一个成熟证据链应同时记录版本、平台、命令、预期观察点与结论边界。本书把这些
信息放进每章的 `examples/*/evidence.md`，自动脚本负责发现工具链升级后的漂移，
而不是把某次输出包装成永久真理。
