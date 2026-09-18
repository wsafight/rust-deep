# 第 16 章：无锁与 atomics — 内存序如何落到具体指令

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch16-atomics
scripts/verify-all.sh ch16
```

## ★ 核心证据：每个 `Ordering` 生成**不同**的指令

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

**讲法**：内存序**不是抽象概念**，它直接决定生成哪条指令。
AArch64 有独立的内存序指令，所以差异是**肉眼可见**的。

### 逐条原始输出（`.evidence/ch16-atomics-lib.O3.s`）

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

## ★ 与第 13 章的交叉印证

`ch13` 的 `Arc::clone` 生成的是 **`ldadd`**（不带 `al` 后缀）——
因为 `Arc` 的引用计数用的是 **`Relaxed`**！

```
ch13: ldadd    ← Arc::clone  → Ordering::Relaxed
ch16: ldadd    ← fetch_add(Relaxed)
      ldaddal  ← fetch_add(SeqCst)
```

**这是一个绝好的教学点**：同一个 `Arc`，clone 用 `Relaxed`、
drop 时才用 `Release` + `dmb ishld`。为什么够用？
因为"引用计数的增减"本身不需要顺序，**只有"最后一次 drop"才需要**——
它必须看到之前所有的写。这就是 `Arc` 的健全性论证的核心。

## ★★ 跨架构对照：这才是本章最有价值的证据

```bash
tools/evidence.sh ch16-atomics x86_64-apple-darwin
```
（`evidence.sh` 第二个参数是 target；产物文件名自动带 `.x86_64` 后缀，
不会覆盖宿主架构的证据。）

| Rust 写法 | AArch64（宿主） | x86_64（对照） |
|---|---|---|
| `load(Relaxed)` | `ldr` | `movq (%rdi), %rax` |
| `load(Acquire)` | **`ldapr`** | `movq (%rdi), %rax` |
| `load(SeqCst)` | **`ldar`** | `movq (%rdi), %rax` |
| `store(Relaxed)` | `str` | `movq %rsi, (%rdi)` |
| `store(Release)` | **`stlr`** | `movq %rsi, (%rdi)` |
| `store(Relaxed)` | `str` | `movq %rsi, (%rdi)` |
| `fetch_add(Relaxed)` | `ldadd` | `lock xaddq` |
| `fetch_add(SeqCst)` | `ldaddal` | `lock xaddq` ← **与 Relaxed 完全相同** |
| `compare_exchange` | `casal` + `cmp`/`cset` | `lock cmpxchgq` + `sete` |

### ★★ 两个**反直觉**的实测结论（写作时最容易写错的地方）

**(A) x86 上 `Relaxed` 的 RMW 和 `SeqCst` 的 RMW 生成完全相同的代码。**
`fetch_add_relaxed` 和 `fetch_add_seqcst` 都是 `lock xaddq` ——
因为 x86 的 `lock` 前缀本身就提供全序，没有"更弱的原子 RMW"。
**在 x86 上对 RMW 放松内存序，性能上一分钱都省不下来。**

**(B) x86 上 load/store 放松内存序也省不下来。**
三个 load 全是 `movq`（含 `Acquire` 和 `SeqCst`），
两个 store 全是 `movq`（含 `Release`）。
**"放松内存序"的收益是架构相关的 —— AArch64 上才有明显差异。**

### 这张表说明了两件事

**(1) x86 的强内存模型让大部分 Ordering 消失。**
三个 load 全是 `movq`，两个 store 全是 `movq` ——
因为 x86 的普通 load 天然有 acquire 语义、普通 store 天然有 release 语义。
**"内存序"不是抽象概念，它是"这个架构需不需要额外指令"的问题。**

**(2) RMW（读-改-写）在任何架构上都必须显式同步。**
`fetch_add` / `compare_exchange` 在 x86 上都需要 `lock` 前缀
（`lock xaddq` / `lock cmpxchgq`），在 AArch64 上需要 `ldaddal` / `casal`。
**这是两个架构唯一"看起来一致"的地方。**

★ 但**失败路径的指令数不同**：AArch64 的 `casal` 之后要
`cmp` + `cset` 才能算出 `bool`（`casal` 把旧值写回寄存器）；
x86 的 `cmpxchgq` **直接**设置零标志位，一条 `sete` 就够。
**同一个语义，两个架构的指令数不同。**

→ **写作建议**：这一节用**并排的两栏代码**，比任何文字都清楚。
这是全书唯一一处"同一份 Rust 代码、两种 ISA、肉眼可见的差异"。

## ★ 与第 13 章的交叉印证

`ch13` 的 `Arc::clone` 生成的是 **`ldadd`**（不带 `al` 后缀）——
因为 `Arc` 的引用计数用的是 **`Relaxed`**！

```
ch13: ldadd    ← Arc::clone  → Ordering::Relaxed
ch16: ldadd    ← fetch_add(Relaxed)
      ldaddal  ← fetch_add(SeqCst)
```

**这是一个绝好的教学点**：同一个 `Arc`，clone 用 `Relaxed`、
drop 时才用 `Release` + `dmb ishld`。为什么够用？
因为"引用计数的增减"本身不需要顺序，**只有"最后一次 drop"才需要**——
它必须看到之前所有的写。这就是 `Arc` 的健全性论证的核心。

## 断言（9 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | Acquire 读生成 `ldapr` | `^\s*ldapr\s` |
| 2 | SeqCst 读生成 `ldar` | `^\s*ldar\s` |
| 3 | Release 写生成 `stlr` | `^\s*stlr\s` |
| 4 | SeqCst fetch_add 生成 `ldaddal` | `ldaddal` |
| 5 | CAS 生成 `casal` | `casal` |
| 6 | Relaxed 读的函数存在 | `^_load_relaxed:` |
| 7 | x86_64 的 SeqCst fetch_add 需要 `lock` 前缀 | `lock\s+xaddq` |
| 8 | x86_64 的 CAS 需要 `lock cmpxchgq` | `lock\s+cmpxchgq` |
| 9 | x86_64 上这些 Ordering **不需要** fence 指令 | `assert_not_contains 'lfence|mfence|sfence'` |

## 待办

- [ ] 补一个"无锁队列/栈"的最小例子（第 16 章的实战部分）
- [ ] ⚠️ 注意：x86_64-apple-darwin **只能看代码，不能运行**（本机是 arm64）
