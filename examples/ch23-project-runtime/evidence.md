# 第 23 章（实战）：mini async runtime — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> Miri：`miri 0.1.0 (923c95cdf5 2026-09-16)`

## 复现命令

```bash
# ★ 运行期演示
cargo run -p ch23-project-runtime
# order  = [0, 1, 2]
# count  = 5
# local  = 42

# ★ 调度行为测试（6 个）
cargo test -p ch23-project-runtime

# ★★ Miri：手写 waker 的引用计数配平
cargo +nightly miri test -p ch23-project-runtime --test scheduler
# 或统一入口：scripts/verify-miri.sh

# ★ 证据
tools/evidence.sh ch23-project-runtime
scripts/verify-all.sh ch23      # 5 条断言
```

## 关键结论与断言（5 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | mini executor 可编译（队列 + 循环） | `.O3.s` 里 `^_run_three:` |
| 2 | ★ `block_on` 路径：`!Send` 的 future 也能跑 | `.O3.s` 里 `^_run_local:` |
| 3 | 任务在堆上（地址稳定） | `run_three` 函数体内 `___rust_alloc` |
| 4 | Arc 引用计数走原子加（第 13 章） | `run_three` 函数体内 `ldadd` |
| 5 | 队列是真的队列 | `.O3.s` 里 `VecDeque.*grow` |

## ★ 整个 runtime 的三个类型

| 类型 | 是什么 |
|---|---|
| `Task` | 一个被 `Pin` 住的 future + 一个回到队列的通道 |
| `Executor` | **一个队列**（`Mutex<VecDeque<Arc<Task>>>`） |
| `YieldNow` | 一个会自我唤醒的 future（演示 waker） |

### `Task` 的三个字段各有来历

```rust
pub struct Task {
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>,
    executor: Weak<Executor>,
}
```

| 字段 | 为什么是这个类型 | 章 |
|---|---|---|
| `Pin<Box<...>>` | 状态机不能移动；`Box` 给稳定地址 | 19 |
| `dyn Future + Send` | `dyn` 统一任务类型；`+ Send` 因为会跨线程 | 21、22 |
| `Mutex<Option<...>>` | `poll` 要 `&mut`，而 waker 拿到的是 `&Task` | 1 |
| `Weak<Executor>` | ★ **打破引用环**（`Executor → Arc<Task> → Arc<Executor>`） | 13 |

★ `Option` 是为了在任务完成时**把 future 丢掉**（释放状态机的内存）。

## ★ 核心：executor 就是这三行

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

**`wake` 的全部含义就是入队**：

```rust
fn wake_task(task: Arc<Task>) {
    if let Some(exec) = task.executor.upgrade() {
        exec.ready.lock().unwrap().push_back(task);   // ← 就这一行
    }
}
```

### 汇编里的三个证据

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

★ 三个断言分别对应 `___rust_alloc`、`ldadd`、`VecDeque...grow`。

## ★★ 本章唯一的 `unsafe`：`RawWakerVTable` 的四个函数

```rust
fn task_waker(task: &Arc<Task>) -> Waker {
    unsafe fn clone(data: *const ()) -> RawWaker { /* 增加引用计数 */ }
    unsafe fn wake(data: *const ()) { /* 消耗一个引用计数 */ }
    unsafe fn wake_by_ref(data: *const ()) { /* 不消耗 */ }
    unsafe fn drop_waker(data: *const ()) { /* 减少引用计数 */ }

    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop_waker);

    let raw = Arc::into_raw(Arc::clone(task)) as *const ();
    // SAFETY: raw 来自 Arc::into_raw，vtable 的四个函数满足上述契约
    unsafe { Waker::from_raw(RawWaker::new(raw, &VTABLE)) }
}
```

### 契约（**必须写进 SAFETY 注释**）

| 函数 | 契约 |
|---|---|
| `clone` | 必须**增加**引用计数，返回一个新的 `RawWaker` |
| `wake` | 必须**消耗**一个引用计数（`from_raw` 的逆操作） |
| `wake_by_ref` | 必须**不消耗**引用计数 |
| `drop` | 必须**减少**引用计数 |

★ **任何一个写错，都是内存泄漏或 use-after-free。**
而编译器**完全无法检查** —— 四个函数的类型是裸的 `*const ()`。

### 为什么类型系统检查不了

**`*const ()` 丢掉了所有信息**：类型系统不知道它是 `Arc<Task>`，
更不知道引用计数该加还是该减。

★ 这与第 19 章那条结论是同一个主题：
**契约不在代码里** —— `addr_of!` 与 `addr_of_mut!` 生成同样的汇编，
一个是 sound 的、一个是 UB。

### 谁能检查它：Miri

```bash
$ cargo +nightly miri test -p ch23-project-runtime --test scheduler
running 6 tests
......
test result: ok. 6 passed; 0 failed
```

Miri 能发现这些测试路径上由错误引用计数引起的泄漏或无效访问，
但通过不等于对所有路径的完整证明。

★ 这是全书里 Miri **最物有所值**的一次 ——
它检查的是一个类型系统完全看不见的契约。

⚠️ 但仍要记住第 25 章的提醒：**Miri 通过不等于 sound**。

## ★ `Pending` 与唤醒协议（本章最容易踩的坑）

`Pending` 只表示本次 poll 尚未完成。如果 future 之后可能取得进展，
它必须注册或安排对当前 waker 的唤醒；按设计永远不完成的 future 可以
合法地永久 Pending。若本来希望继续却漏掉唤醒，任务会静默停在队列外。

本章把它固化成了一个测试：

```rust
#[test]
fn pending_without_wake_is_lost() {
    struct NeverWakes;
    impl Future for NeverWakes {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
            Poll::Pending          // ← 故意不调用 _cx.waker()
        }
    }
    // ...
    exec.run();                    // 立刻返回（队列空了）
    assert!(!*done.lock().unwrap(), "任务被丢掉了（这正是本用例要证明的）");
}
```

对照 `YieldNow`（调用了 waker）：

```rust
#[test]
fn yield_now_wakes_and_finishes() {
    // ... 同样结构，但 YieldNow 调用了 wake_by_ref()
    assert_eq!(*done.lock().unwrap(), 1);      // 完成了
}
```

## 调度行为：6 个测试

| 测试 | 断言 |
|---|---|
| `three_tasks_run_in_order` | `run_three() == [0, 1, 2]` |
| `count_to_n` | `block_on(count_to(5)) == 5`，`count_to(0) == 0` |
| `non_send_future_can_block_on` | ★ 含 `Rc` 的 future 能跑（`== 42`） |
| `pending_without_wake_is_lost` | ★ 漏掉 waker → 任务丢失 |
| `yield_now_wakes_and_finishes` | 调用 waker → 完成 |
| `wake_from_another_thread` | ★ **跨线程唤醒**（waker 在另一个线程被调用） |

★ 最后一个测试是 `Task: Send` 的**唯一理由**：
真实 executor 里，waker 常常在别的线程被调用
（IO 线程、定时器线程、别的 worker）。

## ★ `block_on` 不需要 `Send`（第 22 章的又一次印证）

```rust
pub fn block_on<F: Future>(f: F) -> F::Output { ... }    // ← 没有任何 bound
```

```rust
pub fn run_local() -> u64 {
    block_on(async {
        let r = std::rc::Rc::new(21u64);      // !Send
        YieldNow::new().await;
        *r * 2                                 // → 42
    })
}
```

**编译得过、跑得出 42。** 因为 `block_on` 不跨线程。

★ 又一次印证：**`Send` 的要求来自"交给谁"，不来自 `async`。**

## ★ tokio 在 60 行骨架上加了什么

| tokio 的组件 | 作用 | 本章有没有 |
|---|---|---|
| 多线程 worker 池 | 并行执行任务 | ❌ |
| IO 驱动（epoll/kqueue） | IO 就绪时唤醒任务 | ❌ |
| 定时器驱动 | `tokio::time::sleep` | ❌ |
| `JoinHandle` / 取消 | 拿结果、取消任务 | ❌ |
| `spawn_blocking` | 阻塞线程池（第 22 章） | ❌ |
| **park/unpark** | 没任务时**不忙等**（本章是"队列空就返回"） | ❌ |

★ **注意最后一行**：本章的 `run` 在队列空时**直接返回**。
真实 executor 会 **park**（挂起线程），新任务入队时再 unpark。

**这就是"为什么不要自己写 executor"的完整答案：
骨架 60 行，剩下的全是并发正确性和平台细节。**

## 交叉验证（可选）

```bash
cargo run -p ch23-project-runtime
cargo test -p ch23-project-runtime
cargo +nightly miri test -p ch23-project-runtime --test scheduler
awk '/^_run_three:/{on=1} on{print} on&&/cfi_endproc/{exit}' \
  .evidence/ch23-project-runtime-lib.O3.s | grep -E 'rust_alloc|ldadd|grow'
```

## 待办

- [x] 5 条断言全绿（`verify-all.sh ch23`）
- [x] 6 个调度行为测试（含"漏掉 waker 就丢任务"和"跨线程唤醒"）
- [x] 接入 `scripts/verify-miri.sh`（手写 waker 的引用计数配平）
- [x] 运行期演示三条输出
- [ ] 第四部分（18–23）至此全部完成
- [ ] 可以考虑加一个"park/unpark"版本，演示不忙等的 executor
