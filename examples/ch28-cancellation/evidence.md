# 第 28 章：异步取消安全 - 实测证据

> 最后验证：**rustc 1.99.0** / Tokio 1.53.1 / `aarch64-apple-darwin`

## 运行期证据

```bash
cargo test -p ch28-cancellation
```

| 场景 | 结果 |
|---|---|
| `JoinHandle::abort` 后等待 handle | future 内的 guard 析构 1 次 |
| `select!` 的 pending 分支落败 | loser 内的 guard 析构 1 次 |
| `timeout` 到期 | inner future 的 guard 析构 1 次 |
| 先 `pop` 再 `.await`，随后取消 | 队列从 `[7]` 变成 `[]` |
| 先 `.await` 再 `pop`，随后取消 | 队列仍为 `[7]` |
| 循环内重建 `read_pair` | 两轮取消消费掉 1、2，随后只能读到 `[3, 4]` |
| 外部保存并 pin 同一 `read_pair` | 落败后继续 poll，读到 `[1, 2]` |
| `watch` 协作停机后 await handle | 正常退出且 guard 析构 1 次 |

前三组证明“取消的机械动作就是 drop future”。最后两组证明 drop 只能释放
状态机持有的资源，不能撤销已经提交到其他对象或外部系统的副作用。

重建与保留读取的对照使用 `biased;` 和 channel 的 Pending 状态构造确定性交错，
不依赖定时器精度。`fail/await_in_drop.rs` 另验证 Drop 内 await 报 E0728；
这不是业务取消安全的类型证明，而是说明可靠异步清理需要显式异步接口。

## MIR 证据

```bash
tools/mir.sh ch28-cancellation
grep -n 'DropSignal\|Pending' .evidence/ch28-cancellation-lib.mir
```

`wait_forever` 的挂起变体同时保存 `DropSignal` 与 `pending()` 返回的 future。
executor 丢弃这个状态机时，生成的 coroutine drop glue 按已初始化字段执行析构。

`remove_then_wait` 的状态机还保存已经从队列取出的 `item`。取消会释放这个值，
但 `Vec::pop` 对共享队列造成的修改已经发生，drop glue 没有对队列的访问权，
自然无法回滚。

## 工程含义

- `abort()` 发出请求，不代表清理已经结束；需要 `.await` handle 观察完成；
- `select!` 和 `timeout` 都会取消未完成的 future；
- cancellation safety 是操作级契约，不是 `Send` 一样的 marker trait；
- 锁、内存和 guard 可以由 RAII 清理，网络写入、队列消费和业务状态需要协议设计；
- `spawn_blocking` 一旦开始执行，普通 async abort 不能停止那段同步代码。
