# 第 17 章（实战）：构造一个并发任务池 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch17-project-threadpool
scripts/verify-all.sh ch17       # 9 条断言（PASS=12）
```

## 关键结论与断言（9 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | 任务类型是 `Box<dyn FnOnce() + Send>` | MIR 里 `Sender::<Box<dyn FnOnce() + Send>>::send` |
| 2 | ★ execute 里发生 unsize 强制转换 | MIR 里 `PointerCoercion(Unsize` |
| 3 | ★ 唯一的共享可变：`Arc<Mutex<Receiver>>` | MIR 里 `Arc::<Mutex<Receiver<Box<dyn FnOnce() + Send>>>>::new` |
| 4 | worker 循环里先 `lock` | MIR 里 `Mutex::<...Receiver...>::lock` |
| 5 | 再 `recv` | MIR 里 `Receiver::<...>::recv` |
| 6 | ★ 然后立刻 drop guard | MIR 里 `drop(_3)` |
| 7 | `call_once` 在 guard drop 之后 | MIR 里 `Box<dyn FnOnce() + Send> as FnOnce<()>>::call_once` |
| 8 | Drop 先关 channel | MIR 里 `Option::<Sender<Box<dyn FnOnce() + Send>>>::take` |
| 9 | Drop 再 join worker | MIR 里 `JoinHandle` |
| 10 | 任务必须是 `Send` | `fail/job_not_send.rs` → **E0277** |

## ★ 核心证据一：任务类型在 MIR 里逐字可见

```mir
_7 = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>() -> [return: bb4, unwind continue];
```

三个约束各有出处：

| 约束 | 为什么 | 来自 |
|---|---|---|
| `FnOnce()` | 任务只执行一次 | 第 6 章 |
| `Send` | 任务跨线程执行 | 第 12 章 |
| `'static` | 任务在线程里存活 | 线程生命周期 |

### 装箱与 unsize（★ 第 7 章的实证）

`execute` 的 MIR（第 21–26 行）：

```mir
bb0: {
    _4 = Box::<F>::new(copy _2) -> [return: bb1, unwind continue];
}
bb1: {
    _3 = move _4 as std::boxed::Box<dyn std::ops::FnOnce() + std::marker::Send> (PointerCoercion(Unsize, Implicit));
}
```

**`PointerCoercion(Unsize, Implicit)` 是显式的** ——
它就是第 7 章"vtable 只在真的 unsize 时生成"的那个构造点。
胖指针多出来的 vtable 指针就在这里产生。

## ★ 核心证据二：锁的作用域（线程池最常见的 bug）

worker 循环的 MIR（`Worker::new::{closure#0}`）：

```mir
bb2: {
    _4 = std::sync::Mutex::<...Receiver<Box<dyn FnOnce() + Send>>>::lock(copy _5) -> [return: bb3, unwind: bb13];
}
bb5: {
    _2 = std::sync::mpsc::Receiver::<Box<dyn FnOnce() + Send>>::recv(copy _7) -> [return: bb6, unwind: bb12];
}
bb6: {
    drop(_3) -> [return: bb7, unwind: bb13];      // ★ guard 在这里 drop（解锁）
}
bb7: {
    _9 = discriminant(_2);
    switchInt(move _9) -> [0: bb10, 1: bb9, otherwise: bb8];
}
bb10: {
    _10 = move ((_2 as Ok).0: Box<dyn FnOnce() + Send>);
    _11 = <Box<dyn FnOnce() + Send> as FnOnce<()>>::call_once(copy _10, const ()) -> [return: bb15, unwind: bb13];
    //   ^^^^ call_once 在 drop(_3) **之后**
}
```

**控制流顺序是**：`lock` → `recv` → **`drop(_3)`** → `call_once`。

**这就是"锁只覆盖 `recv`"的机器证据。**

★ 如果写成"持锁执行任务"：

```rust
let guard = receiver.lock().unwrap();
loop {
    match guard.recv() {
        Ok(job) => job(),        // ❌ 持有锁执行任务 → 所有任务串行化
        Err(_) => break,
    }
}
```

**能编译、能跑对，但线程池退化成单线程。**
这类 bug 类型系统抓不到 —— 只能靠理解"临界区要尽可能短"这条原则。

## ★ 核心证据三：`Drop` 的顺序（死锁陷阱）

```mir
fn <impl at .../src/lib.rs:109:1: 109:25>::drop(_1: &mut ThreadPool) -> () {
    ...
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

★ 为什么必须这个顺序？

1. worker 的循环是 `guard.recv()`；
2. `recv` 在**所有 `Sender` 都被 drop** 之后才返回 `Err`；
3. 如果先 `join`，`recv` 永远不返回 → `join` **永久阻塞** → 程序挂死。

**单独看每条规则都很简单，组合起来就是一个死锁陷阱 ——
而且它编译通过，类型系统帮不了你。**

★ `Option<Sender>` 这个设计也有讲究：`take()` 需要 `&mut`，
所以字段必须是 `Option`（否则无法在 `drop(&mut self)` 里把值移出去）。
**这是 Rust 里"在 `Drop` 中转移所有权"的标准手法。**

## 反例：`fail/job_not_send.rs`

```text
error[E0277]: `Rc<u64>` cannot be sent between threads safely
  --> examples/ch17-project-threadpool/fail/job_not_send.rs:28:51
   |
28 |       let job: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
   |                                                     ^        ------- within this `{closure@...}`
29 | |         tx.send(*r).unwrap();       // ← 捕获了 Rc<u64>
30 | |     });
   | |______^ `Rc<u64>` cannot be sent between threads safely
   |
   = help: within `{closure@...}`, the trait `Send` is not implemented for `Rc<u64>`
```

违反的是三个约束里的第二条（`Send`）。

## 与"每任务一线程"的结构对照

| | 每任务一线程（`spawn_per_task`） | 线程池 |
|---|---|---|
| 线程创建次数 | **N**（任务数） | **n**（worker 数） |
| 队列 | 无（OS 调度） | channel |
| 共享可变状态 | **无** | **一处**（`Arc<Mutex<Receiver>>`） |
| 背压 | 无 | 有 |
| 任务类型 | 任意 `FnOnce + Send` | 任意 `FnOnce + Send` |

**这不是"线程池一定更好"** —— 如果任务数很少，
`spawn_per_task` 更简单、**零共享**、没有队列开销。

> 本书在没有 benchmark 数据之前不写"哪个更快"（PLAN §13 断点 6）。
> 这里只说结构上的差异。

## 交叉验证（可选）

```bash
grep -n 'channel::<Box<dyn FnOnce() + Send>>' .evidence/ch17-project-threadpool-lib.mir
grep -n 'PointerCoercion(Unsize'              .evidence/ch17-project-threadpool-lib.mir
sed -n '/fn <impl at.*Worker/,/^}/p' .evidence/ch17-project-threadpool-lib.mir \
  | grep -nE 'lock|recv|drop\(_3\)|call_once'
sed -n '/fn <impl at.*Drop for ThreadPool/,/bb3/p' .evidence/ch17-project-threadpool-lib.mir
```
