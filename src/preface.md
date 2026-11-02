# 前言

> 官方书教你写 Rust；这本书教你别被 Rust 卡住。

<div class="reading-intro">
  <span class="reading-kicker">RUST 1.99 · EDITION 2024</span>
  <p class="reading-lead">不只让代码编译通过，<br>更要知道它为什么成立。</p>
  <p class="reading-description">30 章，从最小反例走向安全抽象。先建立心智模型，再沿源码、MIR 与运行时证据验证你的判断。</p>
  <a class="reading-start" href="./00-reading-evidence.md">从证据阅读入门 <span aria-hidden="true">→</span></a>
</div>

<nav class="reading-paths" aria-label="按问题开始阅读">
  <a class="reading-path" href="./part1/01-borrow-checker.md">
    <span class="path-category">所有权与借用</span>
    <strong>这段借用为什么过不了？</strong>
    <span class="path-description">从引用的最后一次使用，理解编译器究竟在保护什么。</span>
    <span class="path-chapter">第 1 章 · 借用检查器 <span aria-hidden="true">→</span></span>
  </a>
  <a class="reading-path" href="./part4/19-pin-unpin.md">
    <span class="path-category">异步与地址稳定性</span>
    <strong>Pin 到底固定了什么？</strong>
    <span class="path-description">分清指针句柄与被指向的值，再看 Future 为什么不能随意移动。</span>
    <span class="path-chapter">第 19 章 · Pin 与 Unpin <span aria-hidden="true">→</span></span>
  </a>
  <a class="reading-path" href="./part5/24-unsafe-boundary.md">
    <span class="path-category">unsafe 与 soundness</span>
    <strong>安全接口的边界怎么守？</strong>
    <span class="path-description">把“不出事”的直觉，变成可以逐条核对的安全不变式。</span>
    <span class="path-chapter">第 24 章 · unsafe 的边界 <span aria-hidden="true">→</span></span>
  </a>
  <a class="reading-path" href="./part6/30-project-arrayvec.md">
    <span class="path-category">综合实战与审计</span>
    <strong>怎样写一个安全的容器？</strong>
    <span class="path-description">把初始化、析构和 panic 安全串起来，亲手验证 ArrayVec。</span>
    <span class="path-chapter">第 30 章 · ArrayVec <span aria-hidden="true">→</span></span>
  </a>
</nav>

## 这本书写给谁

写给**已经能写、但总被卡住**的人。你大概已经过了“语法看不懂”那一关，
现在更常见的困惑是：“这段代码明明合理，编译器为什么不同意？”

如果你符合下面任意一条，这本书就是为你写的：

- 语法都会，但一遇到借用检查器报错就靠"试试加个 `clone()`"蒙过去；
- 写泛型 + 生命周期时，编译器报错和你的直觉完全不一致；
- 能写 `async fn`，但说不清 `Pin` 为什么存在、`Send` 为什么"传染"；
- 看过 `unsafe` 的文档，但不敢确定自己写的到底 sound 不 sound；
- 读过官方 *The Rust Programming Language*，也翻过 *Rust for Rustaceans*，但到了异步和 unsafe 的深水区还是断片。

这本书不从变量和 `match` 重新讲起。不过每章都会先用几分钟把关键语法
捋顺，再带进业务场景，最后才下潜到编译器。你不需要先会看 MIR；
读到哪里，就在那一章学会看哪一小块。

## 为什么叫"深水区"

Rust 的难，不在语法，在**心智模型**。

| 区域 | 对应的难点 |
|---|---|
| 浅水区 | 语法、类型、所有权的基本规则 —— 官方书已经讲透 |
| 深水区 | 借用检查器背后的真实规则、生命周期与 variance、trait 系统的组合爆炸 |
| 洋流 | 并发：`Send`/`Sync` 如何让所有权规则在多线程下依然成立 |
| 深渊 | 异步：`Future`、`Pin`、executor，以及它们与生命周期的纠缠 |
| 海沟 | `unsafe`：如何用不安全的代码，构造出安全的抽象 |

这本书只写后四层。

## 怎么读

- 可以按问题跳读，但强烈建议先顺序读完前两部分（所有权 → trait）；
  后四部分会频繁引用这些心智模型。
- 前四部分末尾都有综合实战，第五部分用一套 `unsafe` 决策流程收尾。
  第六部分再把析构、异步取消和内存有效值汇合到一个 `ArrayVec` 实战。
  建议别跳过：读懂规则和拿它做设计，中间还隔着一次真正的取舍。
- 代码都在 **Rust 1.99.0 / edition 2024** 上验证过。

还不会读 MIR 或汇编时，先读[第 0 章](./00-reading-evidence.md)。
每类主张的适用边界见[附录 D.0](./appendix/d-verification.md)。

多数章节按以下学习路径组织，实战章按实际设计步骤展开：

1. **先把语法或工具认清**：用最小篇幅说明“它是什么、解决什么问题”；
2. **放到业务里**：把语法放进服务端、存储、并发、异步或底层组件的真实决策；
3. **一个会让你卡住的例子**：从具体错误或反直觉行为进入问题；
4. **编译器眼里的样子**：沿 MIR、LLVM IR、汇编或 Miri 找证据；
5. **亲手验证与小结**：复跑命令，并把结论收成可迁移的判断规则。

第一次阅读时，可以先读“认清语法 → 放到业务里 → 小结”，建立地图后再回到
证据章节；遇到实际编译错误时，则可以直接从附录 B 按错误码反查正文。

如果你是带着问题来的，可以这样走捷径：

| 你现在卡在哪里 | 建议路线 |
|---|---|
| 借用和生命周期报错 | 1 → 2 → 3；遇到自引用再读 4 |
| trait API 不知道怎么设计 | 6 → 7 → 8，再按需读 9、10，最后做 11 |
| 多线程共享状态 | 12 → 13 → 15；消息传递读 14，原子序读 16 |
| `tokio::spawn` 报 `Send` / `'static` | 18 → 20 → 22 |
| 异步 trait 或运行时 | 19 → 21 → 23 |
| 准备写 `unsafe` | 先读 24，再读 25，最后用 26 做决策 |
| 任务取消后留下半完成状态 | 18 → 27 → 28 |
| `MaybeUninit` 或自定义容器 | 24 → 25 → 27 → 29 → 30 |
| 不知道该用 Miri、Loom 还是 fuzz | 附录 D |

## 版本说明

本书基于：

- **Rust 1.99.0**（stable）
- **Edition 2024**
- 示例代码用 `rustc 1.99.0` / `cargo 1.99.0` / LLVM 23.1.1 验证

> **关于版本**：补丁版本通常保持语言兼容，但诊断文本、MIR、LLVM IR 和汇编
> 都可能变化。正文采用 Rust 1.99.0 口径；具体工具链和平台以附录 A 及
> `examples/*/evidence.md` 的“最后验证”记录为准。

## 一条约定

书中所有 `unsafe` 代码，都会明确写出它依赖的**安全不变式
（safety invariant）**：调用者要保证什么，实现者又维护什么。
这不是装饰性注释；如果某段 `unsafe` 没解释它为什么 sound，那就是这本书的 bug。

## 范围与阅读分层

本书不是语法大全，也不系统教授宏编写、框架选型或操作系统接口。熟悉语法时，
可以从“卡住的例子”开始；建立模型后再展开完整证据。不能把当前样本的指令数
当成跨版本性能保证。

长 MIR、LLVM IR 和汇编片段放在原生折叠块中。第一遍可以保持关闭，先看上下文
和结论；第二遍再展开并对照自己的输出。折叠不依赖额外插件，也不会修改验证代码。

复现分两层：普通测试、Loom 与书稿构建走跨平台门禁；固定指令断言使用
AArch64 macOS 口径，可选 x86_64 macOS 对照只生成代码。不能因此推断“其他
平台无法运行示例”，也不能推断“所有平台底层实现都已经验证”。

---

准备好了吗？从借用检查器真正在检查什么开始。
