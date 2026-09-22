# 16. 无锁与 atomics 入门

> 一句话：内存序既约束编译器重排，也要由目标硬件兑现；
> 在很多小函数里，这种差异可以直接落到不同指令上。
> AArch64 有独立的内存序指令（`ldar` / `ldapr` / `stlr` / `ldaddal`），
> 所以本章选取的不同 `Ordering` 会生成不同指令；
> 而 x86 的强内存模型下，大部分 `Ordering` 会退化成同一条 `mov`
> —— 这是观察“同一份 Rust 代码如何映射到不同 ISA”的代表性案例。

## 先把语法认清

原子类型通过 `load`、`store`、`fetch_add`、`compare_exchange` 等方法修改
共享整数或布尔值。`Ordering` 不决定操作是否原子，而是约束该操作与前后
其他内存访问之间的顺序：`Relaxed` 只关心原子值本身，Release/Acquire
用于发布与观察数据，`SeqCst` 额外要求所有 SeqCst 操作进入一致总序。

原子操作保证的是**这一笔账不会撕成两半**；内存序回答的是：看见这笔账时，
能不能顺带相信旁边几本账也已经更新。两个问题很像，却不能混着答。

### 放到业务里：配置发布与服务停机

纯取消标志只传递一个布尔值时可用 `Relaxed`；若控制线程先写入新配置，
再把 `READY` 置位，worker 看到 READY 后还要读取那份配置，就需要
Release/Acquire 建立 happens-before。两者代码看起来只差一个枚举值，
承诺的却是完全不同的数据可见关系。

```rust
DATA.store(new_version, Ordering::Relaxed);
READY.store(true, Ordering::Release);

if READY.load(Ordering::Acquire) {
    apply(DATA.load(Ordering::Relaxed));
}
```

Acquire 不是让 `READY`“读得更新”，而是让读到这次发布的线程也能据此
观察发布之前的写；这正是业务语义从“一个标志”升级为“发布一批数据”的地方。

## 16.0 一个会让你卡住的例子

你想写一个"停止标志"，让一个线程通知另一个线程退出：

```rust
use std::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);

// 线程 A
while !STOP.load(Ordering::Relaxed) {
    do_work();
}

// 线程 B
STOP.store(true, Ordering::Relaxed);
```

**如果这个原子变量只表达"是否停止"，这段代码通常就是正确的。**
`Relaxed` 保证每次 load/store 都是原子操作，并参与 `STOP` 自己的修改顺序；
编译器不能把循环里的原子 load 当作普通不变量，只在循环外读取一次。

但 `Relaxed` **不能发布其他内存**。如果线程 B 还要先写一份数据，线程 A
看到标志后再读取它，就需要建立同步关系：

```rust
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static DATA: AtomicU64 = AtomicU64::new(0);
static READY: AtomicBool = AtomicBool::new(false);

// 线程 B：先写数据，再发布 ready
DATA.store(42, Ordering::Relaxed);
READY.store(true, Ordering::Release);

// 线程 A：读到这次 Release 写入后，才能据此观察它之前的写
while !READY.load(Ordering::Acquire) {}
assert_eq!(DATA.load(Ordering::Relaxed), 42);
```

`Acquire`/`Release` 解决的是**其他内存操作的先后关系**，不是
"让某个值最终一定被看到"的调度或活性保证。

你会想："那就用 `SeqCst` 吧，最强的那个。" —— 那 **`Relaxed` 是用来干嘛的？**
如果所有地方都写 `SeqCst`，会不会有问题？

**这一章要回答的是"每一种 `Ordering` 到底在保证什么"** ——
而且答案会具体到**指令**。

## 16.1 先把常见说法摆上桌

通常会这样概括：

- `Relaxed`：只保证原子性，不保证顺序；
- `Acquire` / `Release`：当 acquire 读观察到 release 写入的值（或相应的
  release sequence）时，建立 happens-before 关系；
- `SeqCst`：所有线程看到一致的全序，最贵但最直观；
- 经验法则：**先用 `SeqCst`，测出瓶颈再放松**。

这些都对，但"顺序"和"可见性"是抽象词。
**"`SeqCst` 比 `Relaxed` 贵"贵在哪里？** 贵多少？

本章的答案是：**贵在指令上**。而且在不同架构上贵得**完全不一样**。

## 16.2 编译器眼里的样子

### 16.2.1 本章选取的 `Ordering` 在 AArch64 上体现为不同指令

实测（`.evidence/ch16-atomics-lib.O3.s`）：

| Rust 写法 | AArch64 指令 | 含义 |
|---|---|---|
| `load(Relaxed)` | `ldr` | **无**内存序语义，就是普通读 |
| `load(Acquire)` | `ldapr` | acquire-load 专用指令 |
| `load(SeqCst)` | `ldar` | 更强的全序读 |
| `store(Relaxed)` | `str` | 普通写 |
| `store(Release)` | `stlr` | store-release 专用指令 |
| `fetch_add(Relaxed)` | `ldadd` | 原子性在，**无**内存序 |
| `fetch_add(SeqCst)` | `ldaddal` | 原子性 + acquire-release |
| `compare_exchange(SeqCst)` | `casal` + `cmp`/`cset` | 用旧值判断成功与否 |

原始输出：

```asm
_load_relaxed:                 _load_acquire:              _load_seqcst:
	ldr	x0, [x0]        	ldapr	x0, [x0]       	ldar	x0, [x0]
	ret                     	ret                    	ret

_store_relaxed:                _store_release:
	str	x1, [x0]        	stlr	x1, [x0]
	ret                     	ret

_fetch_add_relaxed:            _fetch_add_seqcst:
	mov	w8, #1          	mov	w8, #1
	ldadd	x8, x0, [x0]    	ldaddal	x8, x0, [x0]
	ret                     	ret

_compare_exchange:
	mov	x8, x1
	casal	x8, x2, [x0]
	cmp	x8, x1
	cset	w0, eq
	ret
```

**这就是"内存序是具体的"最直接的画面**：不同的 `Ordering`
生成的是**不同的指令**，不是同一段代码加不同的注释。

★ AArch64 的后缀含义（值得背下来）：

| 后缀 | 语义 |
|---|---|
| （无） | 纯原子，无内存序 |
| `a` | acquire |
| `l` | release |
| `al` | acquire-release |

所以 `ldadd` / `ldadda` / `ldaddl` / `ldaddal` 是**四条不同的指令**，
分别对应四种内存序组合。

### 16.2.2 ★★ 跨架构对照：x86 上大部分 `Ordering` 消失了

```bash
tools/evidence.sh ch16-atomics x86_64-apple-darwin
```

（`evidence.sh` 的第二个参数是 target；产物文件名自动带 `.x86_64` 后缀，
不会覆盖宿主架构的证据。）

| Rust 写法 | AArch64（宿主） | x86_64（对照） |
|---|---|---|
| `load(Relaxed)` | `ldr` | `movq (%rdi), %rax` |
| `load(Acquire)` | **`ldapr`** | `movq (%rdi), %rax` |
| `load(SeqCst)` | **`ldar`** | `movq (%rdi), %rax` |
| `store(Relaxed)` | `str` | `movq %rsi, (%rdi)` |
| `store(Release)` | **`stlr`** | `movq %rsi, (%rdi)` |
| `fetch_add(Relaxed)` | `ldadd` | `lock xaddq %rax, (%rdi)` |
| `fetch_add(SeqCst)` | `ldaddal` | `lock xaddq %rax, (%rdi)` |
| `compare_exchange` | `casal` + `cmp`/`cset` | `lock cmpxchgq` + `sete` |

x86_64 的原始输出：

```asm
_load_relaxed:                 _load_acquire:              _load_seqcst:
	movq	(%rdi), %rax    	movq	(%rdi), %rax   	movq	(%rdi), %rax
	retq                    	retq                   	retq

_store_relaxed:                _store_release:
	movq	%rsi, (%rdi)    	movq	%rsi, (%rdi)
	retq                    	retq

_fetch_add_relaxed:            _fetch_add_seqcst:
	movl	$1, %eax        	movl	$1, %eax
	lock	xaddq	%rax, (%rdi)   lock	xaddq	%rax, (%rdi)
	retq                    	retq

_compare_exchange:
	movq	%rsi, %rax
	lock	cmpxchgq	%rdx, (%rdi)
	sete	%al
	retq
```

**这张表说明了两件事**：

**(1) x86 的强内存模型让大部分 `Ordering` 消失。**
三个 load 全是 `movq`，两个 store 全是 `movq` ——
**包括 `Acquire` 和 `SeqCst`**。

因为 x86 的普通 load 天然有 acquire 语义、普通 store 天然有 release 语义。
（x86 是 TSO 模型：只允许 store-load 重排。）

实测佐证：`grep -cE 'lfence|mfence|sfence'` 在 x86_64 产物里命中 **0 次**
—— 这些函数**一条 fence 都不需要**。

> ★ **所以不能笼统地说"`SeqCst` 一定比 `Relaxed` 多一条指令"。**
> 本章的三个 load 以及 Relaxed/Release store 生成相同的 `movq`；但这里
> **没有测试 SeqCst store**，后者在 x86 上通常需要 `xchg` 等更强实现。
> **内存序的代价是架构相关的** —— 这正是不该凭直觉猜的地方。

**(2) RMW（读-改-写）必须由原子机器操作或等价的原子指令序列实现。**
`fetch_add` / `compare_exchange` 在 x86 上都需要 `lock` 前缀
（`lock xaddq` / `lock cmpxchgq`）；在这份 AArch64 产物中，
Relaxed RMW 是 `ldadd`，SeqCst RMW 是 `ldaddal` / `casal`。

**这是两个架构唯一"看起来一致"的地方。**

★ 注意一个**反直觉**的细节：x86 上
`fetch_add_relaxed` 和 `fetch_add_seqcst` 生成的是**完全相同的代码**
（都是 `lock xaddq`）。

因为 x86 的 `lock` 前缀本身就提供了全序语义 ——
**在 x86 上，`Relaxed` 的 RMW 和 `SeqCst` 的 RMW 一样贵。**
（这与 AArch64 形成鲜明对照：那里 `ldadd` 比 `ldaddal` 便宜。）

### 16.2.3 与第 13 章的交叉印证

`ch13` 的 `Arc::clone` 生成的是 **`ldadd`**（不带 `al` 后缀）——
因为 `Arc` 的引用计数用的是 **`Relaxed`**！

```text
ch13: ldadd    ← Arc::clone  → Ordering::Relaxed
ch16: ldadd    ← fetch_add(Relaxed)
      ldaddal  ← fetch_add(SeqCst)
```

**两条独立路径得到同一个结论** —— 一个从 `Arc` 的实现读出来，
一个从 `fetch_add` 的不同 `Ordering` 读出来。

★ 这是一个绝好的教学点：同一个 `Arc`，**clone 用 `Relaxed`、
drop 的递减用 `Release`，最后一个持有者再执行 acquire fence**。为什么够用？
因为"增加引用计数"本身不发布 payload，**真正销毁对象前才需要同步** ——
它必须看到之前所有的写。这就是 `Arc` 的健全性论证的核心（第 13 章）。

## 16.3 为什么必须这样设计

### 为什么 AArch64 需要专门的指令

AArch64 是**弱内存模型**（weak memory model）：
硬件可以自由重排 load 和 store，只要单线程语义不变。

所以"我需要这个 load 之后的读不能被提到前面"这件事，
**必须用指令显式表达** —— 这就是 `ldar` / `ldapr` 存在的理由。

而 x86 是 **TSO**（Total Store Order）：硬件只允许一种重排
（store 之后的 load 可以提前）。这个模型已经足够强，
所以大部分 `Ordering` 要求**已经被硬件免费满足了**。

> **结论**：`Ordering` 不是"软件层面的约定"，而是
> **"要求硬件提供什么保证"**。
> 强模型架构上要求少，弱模型架构上要求多 ——
> 于是同一个 `Ordering` 在两个架构上的**代价不同**。

### 为什么 `Relaxed` 仍然有用

因为**原子性和内存序是两件事**：

- **原子性**：读到的值不会撕裂（不会读到"半个 `u64`"）；
- **内存序**：这个操作和其他内存操作的**可见顺序**。

`Relaxed` 保证前者以及该原子对象自身的修改顺序，但不为周围的普通内存访问
建立 happens-before 关系。

**什么场景只需要原子值本身？** 例如：

- `Arc::clone` 的引用计数递增（第 13 章；销毁路径另有 Release/Acquire）；
- 统计用的计数器（`metrics.hits.fetch_add(1, Relaxed)`）；
- "这个值是多少"本身就够用的场景（不需要和其他内存建立关系）。

**判据是**：这个值**是否需要"告诉你别的东西也已经可见"**？
不需要 → `Relaxed`。

### 为什么 `SeqCst` 是默认推荐

因为它**最容易推理**：所有 `SeqCst` 操作存在一个**全局一致的总顺序**，
所有线程看到的是同一个顺序。

而 `Acquire` / `Release` 只建立**成对**的关系 ——
"我看到了你的 release，所以我看到了你 release 之前的所有写"。
一旦有多个变量、多个线程，推理就变得容易出错。

> 对不熟悉内存模型的代码，**先用 `SeqCst` 建立一个容易审查的正确版本**
> 是可取的起点；放松前必须重新给出正确性论证。
> 但本章的实测告诉你一个补充：**在 x86 上，某些放松不会改变机器码**
> （本章的 load、Relaxed/Release store 和 `fetch_add` 对照如此）。
> **放松内存序的收益是架构相关的。**

## 16.4 反直觉的点

### 反直觉之一：x86 上多组 Ordering 会生成相同指令

| 操作 | AArch64 | x86_64 |
|---|---|---|
| `load(Relaxed)` → `load(SeqCst)` | `ldr` → `ldar`（**更贵**） | `movq` → `movq`（**一样**） |
| `store(Relaxed)` → `store(Release)` | `str` → `stlr`（**更贵**） | `movq` → `movq`（**一样**） |
| `fetch_add(Relaxed)` → `fetch_add(SeqCst)` | `ldadd` → `ldaddal`（**更贵**） | `lock xaddq` → `lock xaddq`（**一样**） |

这张表只覆盖本章列出的操作。尤其不能从 Relaxed/Release store 的对照
推出 SeqCst store 也相同；后者通常需要 `xchg` 或等价的更强实现。

反过来说：**AArch64 才是那个"放松内存序有收益"的架构**
（这也是为什么 ARM 服务器上这类优化更受关注）。

★ 这条结论**必须靠实测**才能得到 —— 任何关于"内存序开销"的直觉
在跨架构时都不可靠。

### 反直觉之二：`Relaxed` 的 RMW 在 x86 上和 `SeqCst` 一样贵

`fetch_add_relaxed` 和 `fetch_add_seqcst` 在 x86 上生成
**完全相同的 `lock xaddq`**。

因为 x86 的 `lock` 前缀本身就提供全序 —— 没有"更弱的原子 RMW"这种东西。

在这个孤立 `fetch_add` 样本里，放松内存序没有改变 x86 机器码。
但 Ordering 还约束编译器重排，不能仅凭这一段汇编把更弱排序称作
"纯浪费"。在 AArch64 样本里，`ldadd` 与 `ldaddal` 确实不同。

### 反直觉之三：停止标志可以是 `Relaxed`，发布数据不可以

16.0 的纯停止标志只传递一个原子布尔值，`Relaxed` 足够表达这个需求。
原子 load 仍然是一次可观察的原子操作，不能被提升到循环外。

如果标志还承担"看到我就可以读取此前初始化的数据"的职责，情况就不同了：
发布方要用 `Release`，观察方要用 `Acquire`，并且 acquire load 必须读到
对应 release 写入的值，才能把之前的数据写同步过来。

> 判据不是"这个变量是否重要"，而是：**它是否还在发布其他内存。**

### 反直觉之四：CAS 的返回值需要两条指令

```asm
_compare_exchange:
	mov	x8, x1           ; expected
	casal	x8, x2, [x0]     ; 原子比较交换
	cmp	x8, x1           ; 比较旧值和 expected
	cset	w0, eq           ; 相等 → 成功
	ret
```

`casal` 把"实际读到的旧值"写回 `x8`，然后用 `cmp` + `cset`
算出 `bool`。

**所以 `compare_exchange` 的"成功/失败"是算出来的，不是指令给的。**
这也解释了为什么 `compare_exchange` 有两个 `Ordering` 参数
（成功一个、失败一个）—— 两条路径的语义不同。

★ 而 x86 上恰好相反：`cmpxchgq` **直接**设置零标志位，
所以用 `sete` 一条就够了（不需要 `cmp`）。

**同一个语义，两个架构的指令数不同** —— 又一次说明"抽象是泄漏的"。

### 反直觉之五：x86 上这些函数**一条 fence 都没有**

```bash
grep -cE 'lfence|mfence|sfence' .evidence/ch16-atomics-lib.O3.x86_64.s
# → 0
```

**零条。** 因为 x86 的强内存模型让这些操作**天然满足**要求。

这条也是**断言**之一（`assert_not_contains`）：
如果哪天 rustc 或 LLVM 在 x86 上生成了 fence，
说明我们理解错了 —— 断言会亮红灯。

## 16.5 亲手验证

```bash
tools/evidence.sh ch16-atomics                          # 宿主架构（AArch64）
tools/evidence.sh ch16-atomics x86_64-apple-darwin      # 对照架构
scripts/verify-all.sh ch16

# AArch64：每个 Ordering 一条不同的指令
for f in load_relaxed load_acquire load_seqcst store_relaxed store_release \
         fetch_add_relaxed fetch_add_seqcst compare_exchange; do
  echo "--- $f"
  awk "/^_$f:/,/ret/" .evidence/ch16-atomics-lib.O3.s | head -4
done

# x86_64：大部分退化成 movq
for f in load_acquire load_seqcst store_release; do
  echo "--- $f"
  awk "/^_$f:/,/retq/" .evidence/ch16-atomics-lib.O3.x86_64.s | grep movq
done

# x86 上一条 fence 都没有
grep -cE 'lfence|mfence|sfence' .evidence/ch16-atomics-lib.O3.x86_64.s
```

**怎么算验证成功**：

1. AArch64 上三个 load 分别是 `ldr` / `ldapr` / `ldar` —— **三条不同的指令**；
2. AArch64 上 `store_release` 是 `stlr`，而 `store_relaxed` 是 `str`；
3. AArch64 上 `fetch_add(SeqCst)` 是 `ldaddal`，`fetch_add(Relaxed)` 是 `ldadd`；
4. **x86_64 上三个 load 全是 `movq`**，两个 store 全是 `movq`；
5. x86_64 上 `fetch_add` 是 `lock xaddq`、CAS 是 `lock cmpxchgq`；
6. x86_64 产物里 `lfence|mfence|sfence` **出现 0 次**。

```bash
scripts/verify-all.sh ch16      # 9 条断言
```

## 16.6 与 unsafe 的关系

**`Ordering` 是手写并发算法里最容易写错的地方之一**。原子 API 本身是
安全 API，但错误的排序可能破坏算法不变量；如果算法还借助 `unsafe` 访问
普通内存，后果甚至可能升级成数据竞争和 UB。

★ 内存序用错时，症状常常是"在某些机器上、某些负载下、偶尔出错"。
这是最难调试的一类 bug。所以有三条纪律：

1. **不确定时先用 `SeqCst`**。它的语义最容易推理，代价在大多数场景下可接受
   （尤其在 x86 上，实测几乎为零）；
2. **放松时必须写下论证**：为什么这个操作不需要看到别的内存？
   论证要写在注释里，最好配上"如果放松错了会怎样"的说明；
3. **优先用已有的抽象**（`Arc`、`Mutex`、`AtomicU64`），
   而不是自己拼装 `Ordering`。第 13 章的 `Arc` 就是一个
   **经过验证的 `Relaxed` 用法** —— 它的论证已经写好了。

★ 更深一层：`Ordering` 的正确性**不能靠测试验证**。
一个用错内存序的程序可能在你机器上跑一万次都对。
**唯一可靠的手段是论证 + 工具**（比如 Loom 这样的并发模型检查器）。

> 这与本书的"证据链"方法有一个张力：**内存序的 bug 抓不到证据**。
> 所以这一章的证据是"**生成的指令**"，而不是"运行结果"——
> 我们只能证明"编译器理解了我的意图"，不能证明"我的意图是对的"。

## 16.7 小结

- **本章选取的 Ordering 在 AArch64 上体现为不同指令**：
  `ldr`/`ldapr`/`ldar`、`str`/`stlr`、`ldadd`/`ldaddal`、`casal`。
  后缀 `a` = acquire、`l` = release、`al` = 两者。
- **x86 的强内存模型让大部分 `Ordering` 消失**：
  三个 load 全是 `movq`，两个 store 全是 `movq`，
  产物里 **0 条 fence**。
- **★ "放松内存序"的收益是架构相关的**：
  在这份 x86 产物中，三个 load、Relaxed/Release store，以及
  `fetch_add` 的 Relaxed/SeqCst 对照分别生成相同指令；这不包含 SeqCst store。
  **在 AArch64 上才真的有区别。**
- **RMW 必须由原子机器操作或等价的原子指令序列实现**；排序强度仍由
  `Ordering` 决定，不能把"原子 RMW"与"Acquire/Release 同步"混为一谈。
- **纯停止标志可以用 `Relaxed`**；如果标志还要发布其他数据，才需要
  Release/Acquire。判据是：**这个值是否需要顺带发布其他内存？**
- **`SeqCst` 是默认推荐**，因为它的全序语义最容易推理；
  放松时必须写下论证。
- **内存序的 bug 抓不到证据**：只能靠论证 + 工具，
  这一章的证据是"生成的指令"，不是"运行结果"。

下一章是第三部分的收尾：**构造一个并发任务池**——
把 `Send`/`Sync`、`Arc`、`Mutex`、channel、atomics 全部用在一个真实组件上。
