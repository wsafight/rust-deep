# 28. Future 被丢弃时发生什么：取消安全

> 一句话：异步取消就是停止 poll 并 drop future；资源会按 `Drop` 清理，已经发生
> 的副作用却不会自动回滚。

## 先把机制认清

Rust 的 `Future` trait 没有 `cancel()` 方法。executor 取消一个任务时，核心动作
只有两个：不再 poll，然后丢弃保存状态机的 future。

这让取消复用了上一章的析构语义：跨 `.await` 存活的局部变量是状态机字段，
future 被 drop 时，这些字段也会 drop。

但是“内存被释放”和“操作可以安全重试”是两回事。一个 future 可能已经：

- 从队列取走消息；
- 写出半个协议帧；
- 修改一半业务状态；
- 向远端发送请求，但还没收到响应。

drop 状态机不会让这些副作用倒流。

### 放到业务里：超时后直接重试一次读取

假设 `read_frame` 先从 socket 读 4 字节长度，再读取 payload。若 `timeout` 在第二步
取消它，长度已经被消费。直接重试会把 payload 的前 4 字节当成新长度，协议流
从此错位。

这就是 cancellation safety：一个 future 在任意 `.await` 被丢弃后，是否仍能
安全地放弃或重试这个操作。

## 28.0 一个会让你卡住的例子

本章用队列构造同样的问题：

```rust,ignore
async fn remove_then_wait(queue: Arc<Mutex<Vec<u64>>>) {
    let item = queue.lock().unwrap().pop().unwrap(); // 副作用已经发生
    pending::<()>().await;                           // 在这里取消
    consume(item);
}
```

任务取消后，`item` 会被正常析构，所以没有内存泄漏；但队列中的元素已经消失。
若调用方以为“future 没返回就是没发生”，重试就会处理下一条消息。

类型系统不会拒绝这段代码。cancellation safety 是函数的行为契约，和幂等性、
事务性一样，需要 API 作者说明和测试。

## 28.1 `abort` 到底保证什么

本章的 `wait_forever` 在挂起前构造一个 `DropSignal`：

```rust,ignore
pub async fn wait_forever(dropped: Arc<AtomicUsize>, entered: Arc<Notify>) {
    let _guard = DropSignal(dropped);
    entered.notify_one();
    pending::<()>().await;
}
```

测试等待任务确实进入挂起点，再调用 `JoinHandle::abort()`，最后等待 handle：

```text
spawn -> future 被 poll -> guard 构造 -> Pending
abort -> runtime drop future -> guard drop -> JoinError::is_cancelled()
```

实测计数为 1。需要注意：`abort()` 是请求取消；等待 handle 后，才能确认任务已经
停止并完成析构。对于已经开始运行的 `spawn_blocking` 闭包，abort 不能抢占同步
代码，只能阻止尚未开始的任务。

## 28.2 `select!`：胜者完成，其他分支被丢弃

`tokio::select!` 在同一个任务中轮询多个 future。一个分支完成后，宏返回，其他
分支 future 随临时状态一起 drop。

本章让第一个分支构造 guard 后永远 `Pending`，第二个分支立即 `Ready`。使用
`biased;` 固定轮询顺序后，实测 loser 的 guard 恰好析构一次。

```rust,ignore
tokio::select! {
    biased;
    _ = guarded_pending(signal) => unreachable!(),
    _ = ready(()) => {}
}
```

因此放进 `select!` 的操作必须逐项检查取消安全，不能因为整个宏看起来像普通
控制流就默认每个分支都能中途丢弃。

## 28.3 `timeout` 不是后台继续运行

`tokio::time::timeout(duration, future)` 可以理解为 future 与计时器的竞争。
计时器先完成时，包装器返回 `Elapsed`，内层 future 被 drop。

这与“把任务 spawn 到后台，超时后不再等待”不同。后者只放弃 handle，任务可能
继续运行；`timeout` 直接拥有内层 future，返回时会结束它。

本章用永不完成的 inner future 验证：超时之后 guard 已析构一次。

## 28.4 两阶段设计：把提交放到最后一个 await 之后

最直接的改法是把可取消阶段和提交阶段分开：

```rust,ignore
async fn wait_then_remove(queue: Arc<Mutex<Vec<u64>>>) {
    wait_until_ready().await;       // 可取消阶段，不改变共享状态
    let item = queue.lock().unwrap().pop().unwrap();
    consume_without_await(item);    // 提交阶段没有取消点
}
```

取消只能在 `.await` 让出执行权之后被 runtime 观察。最后一个 await 之后的同步
代码会一直执行到返回、panic 或主动阻塞。因此常见设计是：

1. 在临时缓冲区准备数据；
2. 完成所有可能等待的步骤；
3. 用一个不含 `.await` 的短操作提交。

若提交本身必须 await，就需要更强的协议：事务、幂等请求 ID、可恢复状态机、
补偿操作，或者明确规定 future 不可取消。

## 28.5 cancellation-safe 不等于没有副作用

一个操作可以有副作用，同时仍然 cancellation-safe。判断标准通常是：中途取消后，
再次调用能否继续得到语义正确的结果。

| 操作形态 | 常见判断 |
|---|---|
| 等待通知但不消费数据 | 通常安全 |
| 从字节流读取“任意一些字节”到内部缓冲 | 取决于缓冲归谁所有 |
| 精确读取 N 字节到调用者切片 | 中途取消可能已修改部分切片 |
| channel `recv` | 取决于实现在哪个时刻真正消费消息 |
| 外部支付/写请求 | 通常需要幂等键或事务协议 |

不能只凭函数名判断，必须读 API 的 cancellation safety 文档。

## 28.6 结构化并发减少失控任务

取消问题常被“fire-and-forget”放大：handle 被丢掉后，调用者不知道任务是否仍在
运行、何时清理、错误去了哪里。

更稳妥的结构是让任务生命周期受一个明确作用域管理：保存并等待 `JoinHandle`，
使用 `JoinSet` 聚合子任务，在父操作退出时明确 abort 并 drain 结果。结构化并发
不会自动让业务操作可回滚，但会让“谁负责结束任务”变得可追踪。

## 28.7 亲手验证

```bash
cargo test -p ch28-cancellation
tools/evidence.sh ch28-cancellation

# 状态机里跨 await 保存的 guard 与 pending future
grep -n 'DropSignal\|Pending' .evidence/ch28-cancellation-lib.mir

scripts/verify-evidence.sh ch28
```

四组测试分别验证：

1. abort 会 drop 状态机；
2. `select!` 会 drop loser；
3. timeout 会 drop inner future；
4. 先修改再 await 会留下半完成状态，先等待再提交则不会。

## 28.8 小结

- 取消不是异常注入，而是 future 不再被 poll 并最终被 drop；
- RAII 清理状态机拥有的资源，不回滚已经发生的外部副作用；
- `select!` 的 loser、`timeout` 的 inner future 和被 abort 的任务都会面对取消；
- cancellation safety 是 API 行为契约，类型系统通常不能替你证明；
- 最简单的安全结构是“准备阶段可以 await，提交阶段短且不 await”；
- 不能这样拆分时，需要事务、幂等性或显式可恢复协议。
