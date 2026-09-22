# 17. 实战：构造一个并发任务池

> 一句话：这个线程池的**共享可变状态只有一处**（接收端），
> 其余全部是"不共享"。把它写出来，就是把第 12–16 章的判据
> 全部落到一个真实组件上。

## 先把组件认清

任务统一成 `Box<dyn FnOnce() + Send + 'static>`：`FnOnce` 允许任务消费
捕获值，`Send` 允许跨线程移动，`'static` 排除可能提前失效的借用。
worker 通过 channel 取任务，线程池析构时先关闭发送端，再 join 线程。

可以把它想成后厨：`Job` 是订单，channel 是出单口，worker 是厨师。
出单口无界就没有背压；关店时也必须先停止接单，再等厨师做完手里的菜。

### 放到业务里：缩略图和报表批处理

大量短任务若每个都创建 OS 线程，会反复支付线程创建成本；固定 worker 池可
复用线程。生产实现还必须明确队列上限、拒绝策略、panic 隔离和关闭超时。
本章的无界队列适合解释所有权和关停顺序，不应直接当作生产线程池模板。

```rust
pool.execute(move || resize(input, output));
// Drop: 先关闭任务入口，再等待 worker 退出
```

`move` 把任务数据交给 worker；关闭顺序则保证阻塞在 `recv()` 的线程有机会醒来。

## 17.0 需求

实现一个最小的线程池：

```rust
let pool = ThreadPool::new(4);
pool.execute(|| { /* 任务 */ });
// pool 析构时，所有 worker 优雅退出
```

**接口只有三行，但每一行都踩在一个判据上**：

| 代码 | 用到的概念 | 来自 |
|---|---|---|
| `Box<dyn FnOnce() + Send + 'static>` | trait object + `Send` | 第 7、12 章 |
| `mpsc::channel::<Job>()` | 发送即 move | 第 14 章 |
| `Arc<Mutex<Receiver<Job>>>` | 共享可变状态 | 第 13、15 章 |
| worker 用 `move` 捕获 receiver | 所有权转移 | 第 1、14 章 |

**为什么这值得单独一章？** 因为线程池是"并发概念的交汇点"：
你会同时遇到 trait object、`Send`、channel、`Arc`、`Mutex`、`Drop` 顺序。
**任何一个理解错了，代码都会挂或者编译不过。**

## 17.1 任务类型：三个约束缺一不可

```rust
pub type Job = Box<dyn FnOnce() + Send + 'static>;
```

逐个解释这三个约束：

| 约束 | 为什么必须 | 来自 |
|---|---|---|
| `FnOnce()` | 任务只执行一次 —— 执行完就消耗掉了 | 标准闭包 trait |
| `Send` | 任务要**跨线程**（worker 在别的线程里执行它） | 第 12 章 |
| `'static` | 任务在线程里存活，不能借用栈上的东西 | — |

★ **为什么必须是 `Box<dyn ...>`？**（第 7 章）

每个任务的**具体类型都不同**（每个闭包是一个独特的类型）。
而 channel 的 `T` 必须是一个**确定的类型**。
于是把它们统一成 trait object —— 代价是 16 字节的胖指针 + 一次间接调用。

MIR 里逐字可见：

```mir
_7 = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>() -> [return: bb4, unwind continue];
```

### 装箱与 unsize 转换

`execute` 里做了两件事（MIR 第 21–26 行）：

```mir
bb0: {
    _4 = Box::<F>::new(copy _2) -> [return: bb1, unwind continue];
}
bb1: {
    _3 = move _4 as std::boxed::Box<dyn std::ops::FnOnce() + std::marker::Send> (PointerCoercion(Unsize, Implicit));
}
```

**第二行就是第 7 章讲的 unsize 强制转换**：
`Box<F>` → `Box<dyn FnOnce() + Send>`。
`PointerCoercion(Unsize)` 这个标记在 MIR 里是**显式**的 ——
它就是"胖指针多出来的那个 vtable 指针"的来源。

★ 对照第 7 章的实测：**vtable 只在真的发生 unsize 时才生成。**
这里就是那个构造点。

## 17.2 唯一的共享可变状态

```rust
let (sender, receiver) = mpsc::channel::<Job>();
let receiver = Arc::new(Mutex::new(receiver));   // ← 唯一的共享可变
```

**为什么需要 `Arc<Mutex<...>>`？** 因为：

1. 只有一个 `Receiver`（第 14 章实测：`Receiver` 是 `Send` 但 `!Sync`）；
2. 而**每个** worker 都需要访问它。

MIR 里这个类型写得很长，但每个部分都有出处：

```mir
_8 = Arc::<std::sync::Mutex<std::sync::mpsc::Receiver<Box<dyn FnOnce() + Send>>>>::new(move _9)
```

**这是判据表（第 15 章）里唯一用得上层次 3 的地方。**
其余全部是"不共享"：

- 任务本身：`send` 就 move 走了（层次 0）；
- worker 的 id：`usize`，复制（层次 0）；
- 结果：通过另一个 channel 传回来（层次 0）。

### ★ 锁的作用域：线程池最容易写错的地方

```rust
loop {
    let msg = {
        let guard = receiver.lock().unwrap();   // ← 锁在这里获取
        guard.recv()                            // ← recv 在锁内
    };                                          // ← guard 在这里 drop（解锁）
    match msg {
        Ok(job) => job(),                       // ★ 任务在锁**外**执行
        Err(_) => break,
    }
}
```

**如果写成这样就是 bug：**

```rust
// ❌ 所有任务串行化
let guard = receiver.lock().unwrap();
loop {
    match guard.recv() {
        Ok(job) => job(),        // ← 持有锁执行任务！
        Err(_) => break,
    }
}
```

MIR 里可以看到正确的写法（worker 闭包的 bb2–bb10）：

```mir
bb2: {
    _4 = std::sync::Mutex::<...Receiver<Box<dyn FnOnce() + Send>>>::lock(copy _5) -> [return: bb3, unwind: bb13];
}
bb5: {
    _2 = std::sync::mpsc::Receiver::<Box<dyn FnOnce() + Send>>::recv(copy _7) -> [return: bb6, unwind: bb12];
}
bb6: {
    drop(_3) -> [return: bb7, unwind: bb13];      // ★ guard 在这里 drop
}
bb10: {
    _10 = move ((_2 as Ok).0: Box<dyn FnOnce() + Send>);
    _11 = <Box<dyn FnOnce() + Send> as FnOnce<()>>::call_once(copy _10, const ()) -> [return: bb15, unwind: bb13];
    //   ^^^^ call_once 在 drop(_3) **之后** —— 任务在锁外执行
}
```

**`drop(_3)` 在 `call_once` 之前** —— 这就是"锁只覆盖 `recv`"的机器证据。

> 这也是一个**并发设计的通用原则**：
> **临界区要尽可能短。** 锁只用来保护"取任务"这个动作，
> 而不是"执行任务"。

## 17.3 `Drop` 的顺序：本章最微妙的部分

```rust
impl Drop for ThreadPool {
    fn drop(&mut self) {
        drop(self.sender.take());          // ① 关闭 channel
        for w in &mut self.workers {
            if let Some(h) = w.handle.take() {
                let _ = h.join();          // ② 等 worker 退出
            }
        }
    }
}
```

**为什么必须这个顺序？**

- worker 的循环是 `guard.recv()`；
- `recv` 在**所有 `Sender` 都被 drop** 之后才返回 `Err`；
- 如果先 `join`，`recv` 永远不会返回，`join` **永久阻塞** —— 程序挂死。

**所以必须：先关 channel（让 `recv` 返回），再 `join`。**

MIR 里逐字可见：

```mir
bb0: {
    _4 = &mut ((*_1).1: Option<Sender<Box<dyn FnOnce() + Send>>>);
    _3 = Option::<Sender<Box<dyn FnOnce() + Send>>>::take(move _4) -> [return: bb1, unwind continue];
}
bb1: {
    _2 = std::mem::drop::<Option<Sender<Box<dyn FnOnce() + Send>>>>(move _3) -> [return: bb2, unwind continue];
}
bb2: {
    _6 = &mut ((*_1).0: Vec<Worker>);          // ← 到这里才开始处理 workers
    _5 = <&mut Vec<Worker> as IntoIterator>::into_iter(move _6) -> [return: bb3, unwind continue];
}
```

**`take` → `drop` → 然后才遍历 workers。** 顺序在 MIR 里是**控制流**上的先后。

★ 注意 `Option<Sender>` 这个设计：`take()` 需要 `&mut`，
所以字段必须是 `Option`（否则无法在 `drop(&mut self)` 里移出去）。
**这是 Rust 里"在 `Drop` 中转移所有权"的标准手法。**

### 这个顺序问题的本质

它其实是两个事实的组合：

1. **channel 的关闭语义**：所有 `Sender` 被 drop → `recv` 返回 `Err`；
2. **`join` 会阻塞**：直到线程结束。

**单独看每一条都很简单，组合起来就是一个死锁陷阱。**
而且它**编译通过** —— 类型系统帮不了你，只能靠理解。

## 17.4 与"每任务一线程"的对照

```rust
pub fn spawn_per_task(data: Vec<u64>) -> u64 {
    let handles: Vec<_> = data.chunks(4)
        .map(|c| {
            let c = c.to_vec();
            std::thread::spawn(move || c.iter().fold(0u64, |a, b| a.wrapping_add(*b)))
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).fold(0u64, u64::wrapping_add)
}
```

| | 每任务一线程 | 线程池 |
|---|---|---|
| 线程创建次数 | **N**（任务数） | **n**（worker 数） |
| 队列 | 无（OS 调度） | channel |
| 共享可变状态 | **无** | **一处**（`Arc<Mutex<Receiver>>`） |
| 背压 | 无 | **无**（当前使用无界 `mpsc::channel`） |
| 任务类型 | 任意 `FnOnce + Send` | 任意 `FnOnce + Send` |

★ **线程池把"线程创建"的成本从 N 次降到 n 次**，
代价是引入了一处共享可变状态和一次 move。
当前实现使用无界队列；生产实现若需要限制积压，应改用有界
`sync_channel` 或其他有界队列，并明确 `send` 阻塞或失败时的策略。

**这不是"线程池一定更好"** —— 如果 N 很小（比如 4 个任务），
`spawn_per_task` 更简单、**零共享**、没有队列开销。
**线程池的价值在 N 很大且任务很短的时候。**

> 本书在没有 benchmark 数据之前不写"哪个更快"；这里只说结构差异。

## 17.5 亲手验证

```bash
tools/evidence.sh ch17-project-threadpool
scripts/verify-all.sh ch17

# 任务类型（MIR 里逐字可见）
grep -n 'channel::<Box<dyn FnOnce() + Send>>' .evidence/ch17-project-threadpool-lib.mir

# ★ unsize 转换
grep -n 'PointerCoercion(Unsize' .evidence/ch17-project-threadpool-lib.mir

# ★ 锁的作用域：drop(_3) 在 call_once 之前
sed -n '/fn <impl at.*Worker/,/^}/p' .evidence/ch17-project-threadpool-lib.mir | grep -nE 'lock|recv|drop\(_3\)|call_once'

# ★ Drop 顺序：take → drop → 遍历 workers
sed -n '/fn <impl at.*Drop for ThreadPool/,/bb3/p' .evidence/ch17-project-threadpool-lib.mir
```

**怎么算验证成功**：

1. MIR 里有 `channel::<Box<dyn FnOnce() + Send>>` —— 任务类型正确；
2. MIR 里有 `PointerCoercion(Unsize)` —— **第 7 章的 unsize 在真实代码里的样子**；
3. worker 循环里 `drop(_3)`（guard）出现在 `call_once` **之前**
   —— 任务在锁外执行；
4. `Drop` 的 MIR 里 `take` + `drop` 出现在遍历 workers **之前**
   —— 顺序正确；
5. `fail/job_not_send.rs` 报 **E0277**（捕获 `Rc` 的任务被拒）。

```bash
scripts/verify-all.sh ch17      # 10 条断言
```

## 17.6 与 unsafe 的关系

**这个线程池一行 `unsafe` 都没有** —— 这是本章最重要的结论。

它做到的事：

- 任务跨线程（`Send`）；
- 多 worker 竞争同一个接收端（`Arc<Mutex<...>>`）；
- 优雅关闭（`Drop` 顺序）；
- 零数据竞争（类型系统保证）。

**全部由类型系统保证，不需要一行 `unsafe`。**

★ 对照 `std::sync::mpsc` 的内部实现：底层会使用 `UnsafeCell`、原子操作
以及仔细的唤醒和生命周期管理，其中包含经过封装的 unsafe 代码。
**这就是"安全抽象 + 不安全实现"的分层**：

```text
你的代码（线程池）        ← 100% safe
    ↓ 使用
std::sync::mpsc           ← 接口 safe，实现 unsafe
    ↓ 使用
原子操作 / UnsafeCell     ← unsafe
```

**每一层都把自己的 `unsafe` 关在里面**，
所以上层可以用得完全安全。

★ 一个反过来的观察：**如果你发现自己的线程池需要 `unsafe`，
那多半是设计错了。** 常见的"需要 `unsafe`"的信号：

- 想手工管理线程的生命周期（而不是 `join`）→ 用 `scope` 或 `rayon`；
- 想做无锁队列（而不是 channel）→ 那是另一个量级的复杂度（第 16 章）；
- 想绕过 `Send`（比如共享一个非 `Send` 的类型）→ 第 12 章的
  `unsafe impl Send` 必须配论证，而且**大多数情况下有更好的设计**。

## 17.7 小结

- **任务类型 `Box<dyn FnOnce() + Send + 'static>` 的三个约束各有出处**：
  `FnOnce`（调用后消耗任务）、`Send`（第 12 章）、`'static`（线程生命周期）。
  `Box<dyn ...>` 是第 7 章的 trait object —— 因为每个闭包类型都不同。
- **装箱处的 `PointerCoercion(Unsize)` 在 MIR 里是显式的**
  —— 这是第 7 章"vtable 只在真的 unsize 时生成"的实证。
- **整个线程池只有一处共享可变状态**：`Arc<Mutex<Receiver<Job>>>`。
  其余全部是"不共享"（第 15 章判据表的层次 0）。
- **★ 锁的作用域只覆盖 `recv`**：MIR 里 `drop(_3)`（guard）
  出现在 `call_once` **之前**。**锁内执行任务 = 所有任务串行化**，
  这是线程池最常见的 bug。
- **★ `Drop` 的顺序是死锁陷阱**：必须先关 channel（`take` + `drop` sender），
  再 `join` worker。反了就会永久阻塞 —— 而且**编译通过**，
  类型系统帮不了你。
- **`Option<Sender>` 是在 `Drop` 中转移所有权的标准手法**。
- **一行 `unsafe` 都没有** —— 这是"安全抽象 + 不安全实现"分层的结果：
  `mpsc` 把底层 unsafe 封装在安全接口后面，因此线程池调用方不必重复证明。

第三部分到此结束。我们有了完整的并发工具箱：
`Send`/`Sync` 的类型层检查、`Arc` 的原子指令、`Mutex` 的平台差异、
channel 的所有权转移、atomics 的内存序，以及它们的代价对照。

下一部分进入**异步**：`Future` 为什么是惰性的，
`async fn` 展开成什么，以及 `Pin` 为什么必须存在。
