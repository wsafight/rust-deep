# 23. 实战：写一个 mini async runtime

> 一句话：**executor 的全部状态就是一个队列，核心循环只有三行。**
> 剩下的全部复杂度，都在"waker 什么时候被调用"这一件事上。

第 18–22 章把异步的每一块都拆开讲了。这一章把它们拼起来 ——
**从零写一个能跑的 executor**，然后看清 tokio 在上面加了什么。

★ 这一章也是全书**唯一一处 `unsafe` 是不可回避的**：
构造 `Waker` 需要实现 `RawWakerVTable` 的四个函数，
而它们的契约（引用计数配平）**编译器和 MIR 都看不见** ——
只能靠 Miri 和你的论证。

## 23.0 一个会让你卡住的例子

你写了一个最简单的 executor（第 18 章的版本）：

```rust
pub fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = Box::pin(f);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::hint::spin_loop(),   // 忙等
        }
    }
}
```

**它能跑。** 但它只能跑"一个" future，而且忙等烧 CPU。

于是你想把它变成真正的 executor：多个任务、队列、唤醒。

```rust
pub struct Executor {
    ready: Mutex<VecDeque<Arc<Task>>>,
}

impl Executor {
    pub fn run(&self) {
        loop {
            let Some(task) = self.ready.lock().unwrap().pop_front() else {
                return;
            };
            task.poll();
        }
    }
}
```

**看起来对。但有个致命的坑**：如果某个任务的 `poll` 返回 `Pending`
**却不调用 waker**，这个任务就**永远躺在队列外面** ——

**不报错、不 panic、什么都不打印，就是不动。**

这是 executor 实现里最经典的 bug。而 `Task` 那个结构体还带来另一个问题：

```rust
pub struct Task {
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>,
    executor: Weak<Executor>,        // ← 为什么是 Weak 不是 Arc？
}
```

**为什么是 `Weak`？** 因为 `Executor` 持有 `Arc<Task>`，
而 `Task` 如果能拿到 `Arc<Executor>`，就形成了一个**引用环** ——
两者永远不会被释放。

最后一个问题：`waker` 是 `Waker` 类型，它的构造函数是 `unsafe` 的：

```rust
unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) }
```

**这个 `unsafe` 要你承诺什么？**

三个问题，指向同一个东西：**executor 的复杂度全在 waker 上。**

## 23.1 表层解释（官方书会怎么讲）

官方书会说：

- executor 负责 poll future 并在就绪时唤醒它；
- `Waker` 是"怎么被叫醒"的抽象；
- 写一个 executor 需要 `RawWaker` + `RawWakerVTable`；
- 生产环境请用 tokio，不要自己写。

最后一条是对的 —— 但**写一遍是最好的理解方式**。
本章要做的就是把"为什么不要自己写"这句话背后的东西摊开。

## 23.2 编译器眼里的样子

### 23.2.1 完整的 executor：三个类型

`examples/ch23-project-runtime/src/lib.rs`。整个 runtime 只有三个类型：

| 类型 | 是什么 |
|---|---|
| `Task` | 一个被 `Pin` 住的 future + 一个回到队列的通道 |
| `Executor` | **一个队列**（`Mutex<VecDeque<Arc<Task>>>`） |
| `YieldNow` | 一个会自我唤醒的 future（用来演示 waker） |

### 23.2.2 `Task`：三个字段各有来历

```rust
pub struct Task {
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>,
    executor: Weak<Executor>,
}
```

**逐字段读**：

| 字段 | 为什么是这个类型 | 章 |
|---|---|---|
| `Pin<Box<...>>` | 状态机不能移动（第 19 章）；`Box` 给稳定地址 | 19 |
| `dyn Future<Output = ()> + Send` | `dyn` 让不同任务类型统一；`+ Send` 因为任务会跨线程 | 21、22 |
| `Mutex<Option<...>>` | `poll` 要 `&mut`，而 waker 拿到的是 `&Task` | 1 |
| `Weak<Executor>` | ★ **打破引用环** | — |

★ 关于 `Option`：它是为了在任务完成时**把 future 丢掉** ——
否则状态机占的内存要等到 `Task` 本身被释放才还回去。

★ 关于 `Weak`：`Executor` 持有 `Arc<Task>`，如果 `Task` 也持有
`Arc<Executor>`，两者**永远不会被释放**。用 `Weak` 让"任务→executor"
这条边**不增加引用计数**。这是第 13 章那个"引用环"问题在这里的复现。

### 23.2.3 ★ 唯一的 `unsafe`：`RawWakerVTable` 的四个函数

```rust
fn task_waker(task: &Arc<Task>) -> Waker {
    unsafe fn clone(data: *const ()) -> RawWaker { /* 增加引用计数 */ }
    unsafe fn wake(data: *const ()) { /* 消耗一个引用计数 */ }
    unsafe fn wake_by_ref(data: *const ()) { /* 不消耗 */ }
    unsafe fn drop_waker(data: *const ()) { /* 减少引用计数 */ }

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop_waker);

    let raw = Arc::into_raw(Arc::clone(task)) as *const ();
    // SAFETY: raw 来自 Arc::into_raw，vtable 的四个函数满足下面列出的契约
    unsafe { Waker::from_raw(RawWaker::new(raw, &VTABLE)) }
}
```

**四个函数的契约**：

| 函数 | 契约 |
|---|---|
| `clone` | 必须**增加**引用计数，返回一个新的 `RawWaker` |
| `wake` | 必须**消耗**一个引用计数（`from_raw` 的逆操作） |
| `wake_by_ref` | 必须**不消耗**引用计数 |
| `drop` | 必须**减少**引用计数 |

★ **四个函数里的任何一个写错，都是内存泄漏或 use-after-free。**
而编译器**完全无法检查** —— 它们都是 `unsafe fn`，类型是裸的 `*const ()`。

**这是第 24 章那句话最纯粹的形态**：

> **`unsafe` 的义务是维持契约的前提。**
> 而这里的契约是关于**引用计数配平**的，编译器和 MIR 都看不见。

★ **Miri 能检查这件事**（`scripts/verify-miri.sh`）：

```bash
cargo +nightly miri test -p ch23-project-runtime --test scheduler
# 6 passed —— 含一个跨线程唤醒的用例
```

Miri 会跟踪每一次 `Arc` 引用计数的增减，**配平错了它会报**。
这是全书里 Miri "最物有所值"的一次。

### 23.2.4 `wake` 的全部含义：入队

```rust
fn wake_task(task: Arc<Task>) {
    if let Some(exec) = task.executor.upgrade() {
        exec.ready.lock().unwrap().push_back(task);
    }
}
```

★ **这一行就是"唤醒"的全部含义 —— 没有魔法，就是入队。**

（如果 `upgrade` 失败，说明 executor 已经没了，任务被安静地丢掉。
真实 executor 还要处理取消、panic、`JoinHandle` 等，但核心就是这一行。）

### 23.2.5 `run`：整个 executor 就是这三行

```rust
pub fn run(&self) {
    loop {
        let Some(task) = self.ready.lock().unwrap().pop_front() else {
            return;
        };
        let _pending = task.poll();
    }
}
```

```asm
_run_three:
	sub	sp, sp, #272          ; 栈帧（含三个任务的状态机）
	...
	bl	___rust_alloc         ; ★ Box::pin + Arc<Task> 的堆分配
	...
	ldadd	...                ; ★ Arc 的引用计数（第 13 章）
	...
	bl	...VecDeque...grow    ; ★ 队列增长
```

**三个断言都能在汇编里找到**（`verify-all.sh ch23`）：

| 断言 | 说明 |
|---|---|
| `___rust_alloc` | 任务被 `Box::pin` + `Arc` 分配在堆上 |
| `ldadd` | Arc 的引用计数走原子加（第 13 章） |
| `VecDeque...grow` | 队列是真的队列 |

### 23.2.6 `YieldNow`：理解 waker 的最佳样本

```rust
impl Future for YieldNow {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            cx.waker().wake_by_ref();      // ★ 关键一行
            Poll::Pending
        }
    }
}
```

★ 这一行就是"我还没好，但我保证等一下会叫你"。

**如果漏掉 `wake_by_ref()`，任务会永远卡在 `Pending` 上** ——
executor 不会替你检查这件事。这个行为被固化成了一个测试：

```rust
#[test]
fn pending_without_wake_is_lost() {
    // 一个 poll 返回 Pending 但不调 waker 的 future
    // → exec.run() 立刻返回，任务被丢掉
    assert!(!*done.lock().unwrap(), "任务被丢掉了（这正是本用例要证明的）");
}
```

### 23.2.7 运行期演示

```bash
cargo run -p ch23-project-runtime
# order  = [0, 1, 2]
# count  = 5
# local  = 42
```

- `order = [0, 1, 2]` —— 三个任务按入队顺序各被 poll 一次，再按顺序完成。
  **这是调度行为的可断言证据。**
- `count = 5` —— `count_to(5)` 每数一次 yield 一次，
  经过 5 次队列往返。
- `local = 42` —— **一个含 `Rc` 的 `!Send` future 用 `block_on` 跑通了**。

## 23.3 为什么必须这样设计

### 为什么 executor 的核心是"队列 + 唤醒"

因为 `poll` 只有两种结果（第 18 章）：

| 结果 | 含义 | executor 要做什么 |
|---|---|---|
| `Ready(v)` | 完成了 | 丢掉任务 |
| `Pending` | 还没好 | **等** —— 但等到什么时候？ |

`Pending` 的语义是"**现在还不能推进，但我保证将来会叫你**"。
"叫你"就是调用 waker。而 waker 做的事就是**把任务放回队列**。

★ **所以整个 executor 的协议只有一条**：

> `poll` 返回 `Pending` 时，**它必须保证将来某时刻会调用 waker**。

**这条协议无法被类型系统检查** —— 漏掉就是任务静默丢失（23.2.6）。

### 为什么 `Task` 必须 `Send`

因为 **waker 可能在别的线程被调用**。

真实的 executor 里，唤醒来自 IO 线程、定时器线程、别的 worker。
`Waker` 是 `Send + Sync` 的，它内部持有的 `Arc<Task>` 也必须 `Send`。

★ 这与第 22 章的结论**完全一致**：

```rust
pub fn spawn<F>(self: &Arc<Self>, future: F)
where F: Future<Output = ()> + Send + 'static
```

`tokio::spawn` 的 `+ Send` 不是"tokio 的要求" ——
**是所有"任务可能跨线程执行"的 executor 的必然要求。**

### 为什么 `Task` 里是 `Mutex<Option<...>>` 而不是 `RefCell`

因为 `Task` 会被多个线程同时持有（`Arc<Task>`）——
`poll` 在 worker 线程，`wake` 可能在 IO 线程。

`RefCell` 是 `!Sync`（第 12 章），在 `Arc` 里用会直接编译失败。
`Mutex` 是这里唯一的选择。

★ 注意锁的粒度：**这个锁只在 `poll` 期间持有**。
真实的 executor 会做得更细（比如只在拿 `&mut future` 时锁），
但"用一个 `Mutex` 保护 future"是最简单的正确实现。

## 23.4 反直觉的点

### 反直觉之一：`Poll::Pending` 是一条**承诺**，不是"稍后再试"

漏掉 waker 调用的任务会**静默消失** —— 不报错、不 panic、不打印。

★ 这解释了为什么 executor 的实现 bug 特别难查：
**它表现为"程序卡住"，而不是"程序崩溃"**。

真实的 executor 会用各种机制缓解：
tokio 的 `spawn` 会返回 `JoinHandle`，你可以 `await` 它并加超时。

### 反直觉之二：`block_on` **不需要** `Send`

```rust
pub fn block_on<F: Future>(f: F) -> F::Output { ... }
```

**没有任何 bound。** 因为它不跨线程 —— 忙等循环就在调用者的线程上。

实测（`run_local`）：

```rust
pub fn run_local() -> u64 {
    block_on(async {
        let r = std::rc::Rc::new(21u64);      // !Send
        YieldNow::new().await;
        *r * 2
    })
}
```

**编译得过。** 这是第 22 章那条结论的又一次印证：

> **`Send` 的要求来自"交给谁"，不来自 `async`。**

### 反直觉之三：`Weak` 在 executor 里不是可选的

```rust
executor: Weak<Executor>,     // ← 不能是 Arc
```

用 `Arc` 的话，`Executor -> Arc<Task> -> Arc<Executor>` 形成引用环，
**两者永远不会被释放** —— 而且**不会有任何报错**。

★ 这是第 13 章那个"引用环"问题在这里的复现。
区别是：第 13 章的环在数据结构里（父子节点互相持有），
这里的环在**执行模型**里（任务和调度器互相持有）。

### 反直觉之四：写 executor 的难点**不在调度**，在 waker

回头看本章的代码：`run` 只有三行、`spawn` 只有五行。
**`task_waker` 是全文最长、唯一含 `unsafe` 的函数。**

★ 因为：

| 组件 | 编译器能检查吗 |
|---|---|
| 队列、循环、`Pin<Box<...>>` | ✅ 类型系统 |
| `Send` 边界 | ✅ 类型系统 |
| **waker 的引用计数配平** | ❌ **只有 Miri** |

**所以"executor 是 `unsafe` 高发区"这句话是准确的** ——
而这个 `unsafe` 的形态是**引用计数配平**，不是别名。

### 反直觉之五：tokio 在它上面加了什么

本章的 executor 只有 **60 行核心代码**。tokio 在这个骨架上加的是：

| tokio 的组件 | 作用 | 本章有没有 |
|---|---|---|
| **多线程 worker 池** | 并行执行任务 | ❌ |
| **IO 驱动**（epoll/kqueue） | 让"IO 就绪"能唤醒任务 | ❌ |
| **定时器驱动** | `tokio::time::sleep` | ❌ |
| **`JoinHandle` / 取消** | 拿到任务结果、取消任务 | ❌ |
| **`spawn_blocking`** | 阻塞线程池（第 22 章） | ❌ |
| **park/unpark** | 没任务时**不忙等**（本章是"队列空就返回"） | ❌ |

★ **注意最后一行**：本章的 `run` 在队列空时**直接返回**。
真实的 executor 会 **park**（挂起线程），等新任务入队时再 unpark。

这就是"为什么不要自己写 executor"的完整答案：
**骨架 60 行，剩下的全是并发正确性和平台细节。**

## 23.5 亲手验证

```bash
# ★ 运行期演示
cargo run -p ch23-project-runtime
# order  = [0, 1, 2]
# count  = 5
# local  = 42

# ★ 6 个调度行为测试
cargo test -p ch23-project-runtime

# ★★ Miri：验证手写 waker 的引用计数配平（含跨线程唤醒）
cargo +nightly miri test -p ch23-project-runtime --test scheduler

# ★ 证据
tools/evidence.sh ch23-project-runtime
scripts/verify-all.sh ch23      # 5 条断言
```

**怎么算验证成功**：

1. `cargo run` 输出 **`order = [0, 1, 2]`** —— 调度行为可预测；
2. `cargo test` **6 个用例全绿**，其中包括
   `pending_without_wake_is_lost`（证明"漏掉 waker 就丢任务"）；
3. **`cargo +nightly miri test` 全绿** —— 手写 waker 的引用计数是配平的；
4. `run_three` 的汇编里有 `___rust_alloc`、`ldadd`、`VecDeque...grow`
   —— 任务真的在堆上、引用计数真的走原子操作、队列真的会增长。

## 23.6 与 unsafe 的关系

本章是全书**唯一一处 `unsafe` 不可回避**的地方。收成三条：

### 这一处的 `unsafe` 契约是什么

> **`RawWakerVTable` 的四个函数必须配平引用计数。**

这不是"别名规则"，也不是"有效性"，而是一个**纯粹的记账契约**。
它和 `Arc` 的内部记账是同一类（第 13 章），
区别是 `Arc` 由标准库维护，而这里**由你维护**。

### 为什么它无法被类型系统检查

因为四个函数的类型是裸的：

```rust
unsafe fn clone(data: *const ()) -> RawWaker
unsafe fn wake(data: *const ())
```

**`*const ()` 丢掉了所有信息** —— 类型系统不知道它是 `Arc<Task>`，
更不知道引用计数该加还是该减。

★ 这和第 19 章那条结论是同一个主题：

> **`addr_of!` 与 `addr_of_mut!` 生成同样的汇编，一个是 sound 的、一个是 UB。**
> **契约不在代码里。**

### 谁能检查它：Miri

```bash
cargo +nightly miri test -p ch23-project-runtime --test scheduler
```

Miri 会跟踪每一次 `Arc` 引用计数的增减。**配平错了它会报。**

★ 但记住第 25 章的提醒：
**Miri 通过不等于 sound**（Stacked Borrows 仍是实验性的）。
它只是"你犯的错恰好被它看见了"。

### 完整的论证该长什么样

如果要给这段 `unsafe` 写一份健全性论证，应该是：

```text
1. data 的来源：Arc::into_raw(Arc::clone(task)) —— 计数 +1
2. clone(data)  ：from_raw + clone + into_raw（还回原份）→ 计数 +1，符合契约
3. wake(data)   ：from_raw（计数 -1，接管所有权）→ 传给 wake_task 后 drop → 净 0
4. wake_by_ref  ：借一份（不 from_raw 接管）→ 计数不变
5. drop_waker   ：from_raw + drop → 计数 -1
6. 结论：每次 Waker 的创建/克隆/消耗/丢弃都与 Arc 计数一一对应
```

★ **这份论证必须写进代码注释**（本章的 `SAFETY` 注释就是这么写的）——
因为编译器不会替你记住它。

### 最后一个提醒

本章的 executor 是**教学用的**。生产代码请用 tokio（第 22 章）。

理由不是"自己写不出来"，而是**剩下那部分全是并发正确性和平台细节**：
IO 驱动、定时器、取消、panic 传播、park/unpark、公平调度……

★ **"能写"和"该写"是两件事** —— 这也正是第 26 章的主题。

## 23.7 小结

- **executor 的全部状态就是一个队列，核心循环只有三行**：
  取一个就绪任务 → poll → 如果 `Pending` 就等 waker 把它放回来。
- **`wake` 的全部含义就是"入队"** —— 没有魔法。
- **`Poll::Pending` 是一条承诺**："我保证将来会调用 waker"。
  漏掉就是任务**静默丢失** —— 不报错、不 panic、就是不动。
  （本章把它固化成了一个测试。）
- **`Task` 的三个字段各有来历**：
  `Pin<Box<...>>`（第 19 章）、`Mutex<Option<...>>`（`poll` 要 `&mut`）、
  **`Weak<Executor>`（打破引用环，第 13 章）**。
- **★ 本章唯一的 `unsafe` 是 `RawWakerVTable`**：
  四个函数的契约是**引用计数配平**。写错就是内存泄漏或 use-after-free，
  **编译器和 MIR 都看不见** —— 只有 **Miri** 能查。
- **`block_on` 不需要 `Send`**（实测含 `Rc` 的 future 能跑）——
  又一次印证第 22 章：**`Send` 的要求来自"交给谁"，不来自 `async`**。
- **写 executor 的难点不在调度，在 waker**：
  队列和循环有类型系统保护，waker 的记账没有。
- **tokio 在 60 行骨架上加的是**：多线程 worker 池、IO 驱动、定时器、
  取消、`spawn_blocking`、park/unpark。
  **"能写"和"该写"是两件事。**

第四部分到这里结束。我们走过了：

```text
18  Future 是惰性的      → 状态机 + poll
19  Pin / Unpin          → 状态机不能移动
20  async 生命周期       → 跨 await 的字段决定 Send
21  AFIT                 → trait 里的 async 撞上 dyn 和 Send
22  tokio                → Send + 'static 的工程形态
23  mini runtime         → 把上面全部拼起来（含唯一的 unsafe）
```

第五部分进入 `unsafe` 与 soundness —— 从第 23 章这个"引用计数配平"
的小例子，走向第 24 章的"元数据契约"、第 25 章的"权限栈"，
以及第 26 章的"什么时候根本不该写"。
