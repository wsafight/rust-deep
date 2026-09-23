# 15. 共享可变状态的所有权设计

> 一句话：**`Arc<Mutex<T>>` 是通用方案，但不该成为不经思考的默认答案。**
> 本章比较五种常见路线，从"完全不共享"到"谁都能改"，
> 不同方案付出的同步、分配和争用成本不同。
> 设计的目标是**用最紧的约束满足需求**，而不是直接用最灵活的那个。

## 先把工具认清

`move` 转移独占所有权，`Arc<T>` 提供共享只读所有权，`Arc<Mutex<T>>`
提供同步的共享可变访问，`AtomicU64` 等原子类型适合单个值的简单并发操作。
另一条常被忽略的路线是每线程保存局部状态，最后再归并。

这里最值钱的经验不是记住四种类型，而是先问：**真的非共享不可吗？**
最便宜的锁永远是没有那把锁，最容易维护的竞争条件则是根本没有竞争。

### 放到业务里：请求计数与聚合报表

全局请求数只需 `AtomicU64::fetch_add`；必须同时更新金额、笔数和最后时间时，
用一把 `Mutex<Stats>` 更容易维护复合不变量；高吞吐批处理则可让每个 worker
维护局部统计，批次结束后归并。先判断是否必须共享，再决定同步原语，通常比
先写 `Arc<Mutex<_>>` 更可靠。

```rust,ignore
requests.fetch_add(1, Ordering::Relaxed); // 单字段统计
stats.lock().unwrap().record(cost, now);  // 多字段不变量
```

两行都“线程安全”，但保护的业务语义不同：前者只保一个数，后者保一组关系。

## 15.0 一个会让你卡住的例子

你要写一个多线程的计数器，很自然地写成：

```rust,ignore
let counter = Arc::new(Mutex::new(0u64));
let mut handles = vec![];
for _ in 0..4 {
    let c = Arc::clone(&counter);
    handles.push(std::thread::spawn(move || {
        for _ in 0..1000 {
            *c.lock().unwrap() += 1;
        }
    }));
}
```

在本章当前工具链生成的循环体里，每次自增涉及约 11 条指令和两次函数调用。

而如果换成 `AtomicU64`：

```rust,ignore
let counter = Arc::new(AtomicU64::new(0));
// ...
counter.fetch_add(1, Ordering::Relaxed);
```

对应 atomic 样本的循环体是 4 条指令。

指令条数明显不同，但这不是吞吐量的直接倍数。它提醒我们：
`Mutex` 提供的能力远超过单个计数器需要的能力。

**问题是：怎么知道什么时候该用哪个？**
这一章要给出一张可操作的判据表。

## 15.1 先把常见方案摆上桌

通常会这样概括：

- 共享可变状态用 `Arc<Mutex<T>>`；
- `Mutex` 提供互斥访问，`Arc` 提供共享所有权；
- `RwLock` 在读多写少时更好；
- 小心死锁（多个锁的加锁顺序要一致）。

这些都对，但它**只讲了一个方案**（`Arc<Mutex<T>>`），
然后讨论"怎么把它用好"。**它没告诉你"什么时候不该用它"。**

本章的结构不同：先列**五种路线**，再给出**选择判据**。

## 15.2 五种路线及其代价

### 15.2.1 设计 1：独占所有权（move）—— 代价为零

```rust
pub fn owned_sum(v: Vec<u64>) -> u64 {
    let h = std::thread::spawn(move || v.iter().fold(0u64, |a, b| a.wrapping_add(*b)));
    h.join().unwrap()
}
```

数据被 **move** 进闭包，线程**独占**它。

**没有任何共享 → 不需要任何同步原语。** 汇编里就是普通的循环。

> 这是第 14 章"发送即 move"的直接应用。
> **"不共享"永远是最便宜的方案** —— 这一条在下面每个设计里都会重复。

### 15.2.2 设计 2：`Arc<T>` 只读共享 —— 一次原子加

```rust,ignore
pub fn shared_read_sum(v: Arc<Vec<u64>>) -> u64 {
    let v2 = Arc::clone(&v);                    // ← 一次 ldadd
    let h = std::thread::spawn(move || v2.iter().fold(0u64, |a, b| a.wrapping_add(*b)));
    let r = h.join().unwrap();
    r + v.len() as u64                          // ← v 仍然可用
}
```

多个线程**只读**共享同一份数据。

**代价**：每个 `Arc::clone` 一次 `ldadd`（第 13 章）。
**但没有锁** —— 只读意味着"多线程同时访问"天然安全。

★ 注意这里体现了 `Arc<T>: Send + Sync` 的条件（第 12 章）：
`T = Vec<u64>` 本身是 `Send + Sync`，所以 `Arc<Vec<u64>>` 也是。
**责任最终落在 `T` 上。**

### 15.2.3 设计 3：`Arc<Mutex<T>>` —— 真正需要共享可变时

```rust,ignore
pub fn mutex_accumulate(a: Arc<Mutex<u64>>, n: u64) -> u64 {
    for _ in 0..n {
        let mut g = a.lock().unwrap();
        *g = g.wrapping_add(1);
    }
    *a.lock().unwrap()
}
```

**这是最贵的方案。** 每次迭代的汇编（`-O`）：

```asm
LBB42_1:
	subs	x20, x20, #1        ; 循环计数
	b.eq	LBB42_16
	ldapr	x0, [x21]           ; ★ pthread Mutex 的 OnceBox 初始化状态
	cbz	x0, LBB42_9
LBB42_3:
	bl	__RNv...pal4unix4sync5mutexNtB2_5Mutex4lock    ; ★ 函数调用
	ldr	x8, [x22]           ; ★ 读 GLOBAL_PANIC_COUNT（poison 检查）
	tst	x8, #0x7fffffffffffffff
	b.ne	LBB42_10            ; 有 panic 过 → 走 poison 路径
	ldrb	w8, [x19, #24]      ; 检查 poison 标志
	cbnz	w8, LBB42_26
	ldr	x8, [x19, #32]      ; 读数据
	add	x8, x8, #1          ; ← 真正的工作：就这一条
	str	x8, [x19, #32]      ; 写数据
LBB42_7:
	...
	bl	__RNv...Mutex6unlock  ; ★ 又要一次函数调用
```

**11 条指令 + 两次函数调用，只为做一次 `+1`。**

★ 而且这里还不包括：

- `Mutex4lock` / `Mutex6unlock` **函数体内部**的指令
  （pthread 调用、错误检查、fast path 判断）；
- **异常安全代码**（如果 `+1` 之间 panic，guard 必须解锁）；
- poison 记账（实测 `GLOBAL_PANIC_COUNT` 在这个文件里出现 **4 次**）。

`ldapr` + `cbz` 在这份 macOS 产物中首先反映的是 pthread Mutex 的懒初始化
包装，不能单独拿来证明锁本身的无竞争 fast path。真正的等待策略在 pthread
实现内部；竞争严重时可能进入内核等待，但不能把每次竞争都等同于一次系统调用。

### 15.2.4 设计 3'：同样的需求，用 atomic 就够了

```rust,ignore
pub fn atomic_accumulate(a: Arc<AtomicU64>, n: u64) -> u64 {
    for _ in 0..n {
        a.fetch_add(1, Ordering::Relaxed);
    }
    a.load(Ordering::Relaxed)
}
```

**循环体（`-O`，完整）：**

```asm
LBB38_2:
	add	x9, x0, #16
	ldadd	x8, x9, [x9]        ; ← 全部的工作
	subs	x1, x1, #1
	b.ne	LBB38_2
```

**4 条指令。没有函数调用、没有 poison 检查、没有异常安全代码。**

对照：

| | 每次自增的循环体 |
|---|---|
| `Arc<Mutex<u64>>` | **11 条指令 + 2 次函数调用**（`lock` / `unlock`） |
| `Arc<AtomicU64>` | **4 条指令** |

★ **这是本章最重要的一条实用建议**：
**先问"共享的是什么"，再问"要不要锁"。**

计数器、标志位、指针、简单的状态机 —— 这些都有**原子类型**，
不需要 `Mutex`。`Mutex` 的价值在于"保护**任意**状态"，
而"任意"是要花钱的。

### 15.2.5 设计 4：每线程一份 —— 零共享

```rust,ignore
pub fn chunked_sum(v: &[u64]) -> u64 {
    let n = available_parallelism();
    let chunk = v.len().div_ceil(n).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = v.chunks(chunk)
            .map(|c| s.spawn(move || c.iter().fold(0u64, |a, b| a.wrapping_add(*b))))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).fold(0u64, u64::wrapping_add)
    })
}
```

每个线程处理**自己的一段**，最后合并。

**线程内部完全没有同步** —— 只在最后做一次加法。

> 这是**并行归约**（map-reduce）的形状，也是最容易被人忽略的方案：
> 很多人第一反应是"共享一个累加器 + 锁"，
> 而正确答案往往是"各算各的，最后合并"。
>
> `std::thread::scope` 让这件事变简单了（不需要 `Arc`，
> 因为作用域保证了生命周期）。

### 15.2.6 反例：`Rc<RefCell<T>>` 不能跨线程

单线程里 `Rc<RefCell<T>>` 是"共享可变"的标准写法。
搬到多线程**立刻报错**（`fail/rc_refcell_thread.rs`）：

```text
error[E0277]: `Rc<RefCell<u64>>` cannot be sent between threads safely
   |
32 |       std::thread::spawn(move || {
   |       ------------------ ^------
   | |_____^ `Rc<RefCell<u64>>` cannot be sent between threads safely
   |
   = help: within `{closure@...}`, the trait `Send` is not implemented for `Rc<RefCell<u64>>`
```

**这不是"`Rc<RefCell>` 不好"，而是"它的设计目标就是单线程"。**

| | `Rc<RefCell<T>>` | `Arc<Mutex<T>>` |
|---|---|---|
| 引用计数 | 普通 `add`（1 条） | 原子 `ldadd` |
| 借用检查 | **运行时**计数（违反时 panic） | 阻塞等待 + poison |
| 能跨线程 | ❌ | ✅ |

★ 注意 `src/lib.rs` 的 `local_refcell` **编译通过**，
而且它的函数体只有 **12 条指令** ——
`Rc<RefCell<T>>` 在单线程里比 `Arc<Mutex<T>>` **便宜得多**。

**选对工具的前提是选对"共享范围"。**

## 15.3 判据：用类型表达"谁能改"

把上面的路线整理成一张表。**按约束从紧到松排列**：

| 层次 | 类型 | 谁能改 | 代价 | 什么时候用 |
|---|---|---|---|---|
| **0** | `T`（move） | 唯一的持有者 | 无共享同步开销 | 数据是"一次性"的 |
| **1** | `Arc<T>` | **没人**改 | 一次 `ldadd` | 读多写无 |
| **2** | `Arc<AtomicU64>` | 共享，但只有"一个值"可改 | 4 条指令 | 计数器 / 标志位 |
| **3** | `Arc<Mutex<T>>` | 谁都能改**任意**状态 | 11+ 条指令 + 函数调用 | 真的需要共享可变 |
| **4** | `Arc<RwLock<T>>` | 读并发、写独占 | 更贵 | 读远多于写**且**状态复杂 |

★ **每一次"升级"都在放宽约束**（能做更多事），
同时**付出代价**（更多的同步指令）。

**设计的目标是"用最紧的约束满足需求"。**

★ 一个实用的决策顺序：

1. **能不能不共享？** （move / channel / 每线程一份）→ 优先层次 0；
2. **共享之后要改吗？** 不改 → `Arc<T>`；
3. **要改，但只是一个整数 / 标志？** → `AtomicXxx`；
4. **要改任意状态？** → `Arc<Mutex<T>>`，并考虑分片（sharding）。

## 15.4 反直觉的点

### 反直觉之一：`Mutex` 保护一个整数，比 `AtomicU64` 贵得多

| | 循环体 |
|---|---|
| `Arc<Mutex<u64>>` | 11 条指令 + 2 次 `bl` |
| `Arc<AtomicU64>` | 4 条指令 |

**贵的不只是"原子操作"**，而是 `Mutex` 提供的**全套记账**：

- `ldapr` + `cbz`（pthread Mutex 的懒初始化检查）；
- `bl ...Mutex4lock`（函数调用 → 可能进 pthread）；
- `GLOBAL_PANIC_COUNT` 的读 + `tst` + 分支（**poison 检查**）；
- poison 标志位检查；
- 异常安全代码（`MutexGuard::drop` 要解锁）；
- 最后 `bl ...Mutex6unlock`。

**"我只是想加个 1" —— 但 `Mutex` 不知道你想干什么，
它必须为"任何事"做好准备。**

### 反直觉之二：`AtomicU64` 不需要"锁"，但它也不是"无代价"

`fetch_add` 生成 `ldadd` —— **仍然是一条原子 RMW 指令**，
在硬件层面是"独占缓存行"的操作。

**所以"用 atomic 代替 Mutex"不是"消除开销"，是"选一个更小的开销"。**
不共享没有共享同步开销，但任务创建、数据搬移或归并本身仍可能有成本。

> `chunked_sum`（每线程一份）避开了热循环中的共享同步；它是否最快，
> 仍取决于数据量、线程创建、缓存局部性和归并成本。

### 反直觉之三：`thread::scope` 让"不共享"变得更容易

以前要在线程间共享数据，必须 `Arc`（因为 `spawn` 要求 `'static`）。
`thread::scope` 之后，**借用**也能进线程：

```rust,ignore
std::thread::scope(|s| {
    s.spawn(|| v.len());        // ← 直接借用 v，不需要 Arc
});
```

**这直接消掉了一层 `Arc` 的代价。**
很多原本需要 `Arc<Vec<T>>` 的地方，现在只需要 `&[T]`。

### 反直觉之四：`RwLock` 常常不是优化，是退化

"读多写少就用 `RwLock`"是一句流行的建议，但它有前提：

- `RwLock` 的**读锁也要原子操作**（通常比 `Mutex` 的 fast path 更重）；
- 写者饥饿 / 读者饥饿的问题需要实现者处理；
- 在**单核或低竞争**场景下，`RwLock` 往往**比 `Mutex` 慢**。

**判据是"读操作的临界区有多长"**，不是"读写的比例"。
如果临界区只有几条指令，`RwLock` 的记账成本会吃掉全部收益。

> 本书没有接入对应 benchmark，因此这里只比较结构和生成代码，
> 不把静态指令数直接换算成吞吐量。

## 15.5 亲手验证

```bash
tools/evidence.sh ch15-shared-state
scripts/verify-all.sh ch15

# ★ 两种累加器的循环体对照
awk '/^_atomic_accumulate:/,/cfi_endproc/' .evidence/ch15-shared-state-lib.O3.s

# Mutex 的记账（poison 检查）
grep -c 'GLOBAL_PANIC_COUNT' .evidence/ch15-shared-state-lib.O3.s

# 反例
rustc --edition 2024 --crate-type=lib examples/ch15-shared-state/fail/rc_refcell_thread.rs
```

**怎么算验证成功**：

1. `atomic_accumulate` 的循环体是 **4 条指令**（`add` / `ldadd` / `subs` / `b.ne`）；
2. `mutex_accumulate` 的循环体里有 `bl ...Mutex4lock` 和 `bl ...Mutex6unlock`
   **两次函数调用**，以及 `GLOBAL_PANIC_COUNT` 的 **poison 检查**；
3. `local_refcell` 的函数体只有 **12 条指令**
   —— `Rc<RefCell>` 在单线程里便宜得多；
4. `fail/rc_refcell_thread.rs` 报 **E0277**。

```bash
scripts/verify-all.sh ch15      # 断言数见 evidence.md
```

## 15.6 与 unsafe 的关系

这一章**几乎没有 `unsafe`** —— 但它是"什么时候**需要** `unsafe`"的前置知识。

★ 层次表里的每一层，都是"用类型表达约束"的实例：

| 层次 | 表达的不变量 |
|---|---|
| `T`（move） | "只有我能访问" |
| `Arc<T>` | "谁都不能改" |
| `Arc<AtomicU64>` | "能改，但改的是**单个原子值**" |
| `Arc<Mutex<T>>` | "能改任意状态，但**一次只能一个**" |

**这些不变量都由类型系统保证，不需要 `unsafe`。**

★ 反过来说：**当你发现这些层次都表达不了你的需求时，
才需要考虑 `unsafe`。** 比如：

- 需要"多个读者 + 一个写者，且写者能就地改" → 这是 `RwLock` 或 `unsafe`；
- 需要"无锁的复杂数据结构" → 只有 `unsafe`（第 16 章的无锁队列）；
- 需要"跨线程共享一个非 `Send` 的类型" → `unsafe impl Send`
  （第 12 章的 `MyBox`），但必须给出论证。

**判据的顺序是**：先用类型表达 → 表达不了 → 才考虑 `unsafe`，
并且要能说清"为什么类型系统表达不了"。

> 第 24 章的 `unsafe` 边界哲学，在这一章就有了伏笔：
> **`unsafe` 不是"绕过类型系统"，是"承担类型系统无法表达的证明义务"。**

## 15.7 小结

- **五种路线的同步代价逐步增加**；当前样本从无共享同步，
  到约 11 条指令 + 2 次函数调用：
  move（零）→ `Arc<T>`（一次 `ldadd`）→ `Arc<AtomicU64>`（4 条）
  → `Arc<Mutex<T>>`（11+ 条 + 两次调用）。
- **`Arc<Mutex<T>>` 是通用方案，但不该是不经分析的默认选择。**
  它提供"保护任意状态"的能力，而"任意"是要花钱的。
- **先问"共享的是什么"，再问"要不要锁"**：
  计数器 / 标志位用 `AtomicXxx`，不要用 `Mutex`。
- **`Mutex` 的成本不只是临界区里的数据修改**：初始化检查、poison 检查、
  异常安全代码、两次函数调用 —— 实测 `GLOBAL_PANIC_COUNT` 出现 4 次。
- **`thread::scope` 消掉了 `Arc` 的那一层**：借用也能进线程。
- **`Rc<RefCell<T>>` 在单线程里便宜得多**（12 条指令），
  但它**不能**跨线程。**选对工具的前提是选对"共享范围"。**
- **最便宜的方案永远是"不共享"**：move / channel / 每线程一份。

下一章深入最底层：**内存序如何落到具体指令**，
以及同一份 Rust 代码在 AArch64 和 x86_64 上生成什么不同。
