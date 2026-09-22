# 14. 消息传递：channel 与所有权转移

> 一句话：`send` 之所以能让编译器"切断"本地对值的访问，
> 不是因为它做了什么运行时检查，而是因为**它的签名是 `fn send(&self, t: T)`**
> —— `T` 按值传递，于是 MIR 操作数明确标成 `move`。
> **"发送即 move"是字面意义上的。**

## 先把语法认清

`let (tx, rx) = mpsc::channel::<T>()` 创建发送端和接收端；`tx.send(value)`
按值接收 `T`，所以非 `Copy` 值的所有权会进入队列。克隆 `Sender` 可产生多个
生产者；所有发送端销毁后，阻塞中的 `recv()` 返回断开错误。

channel 可以看成一条**所有权传送带**：消息放上去以后，发送者手里就没了；
接收者拿到的不是副本，而是那份值本身。队列是否有界，则决定传送带塞满时
生产者要不要停下来。

### 放到业务里：日志与后台写入流水线

请求线程可以把拥有所有权的 `LogEvent` 发给专用写入线程，随后立即处理下一
请求。消费者独占事件，不需要和生产者共享 `&mut LogEvent`。若流量可能超过
写入能力，应使用有界 channel 明确背压；无界 channel 只把拥塞转化成内存增长。

```rust
let (tx, rx) = std::sync::mpsc::sync_channel::<LogEvent>(1024);
tx.send(event)?; // 队列满时阻塞，压力不会无限变成内存占用
```

有界队列把系统容量写进结构里；是否愿意阻塞，则是调用方必须明确的业务策略。

## 14.0 一个会让你卡住的例子

```rust
use std::sync::mpsc;

let (tx, rx) = mpsc::channel::<String>();
let s = String::from("hello");

tx.send(s).unwrap();
println!("{s}");          // ← 编译不过
```

```text
error[E0382]: borrow of moved value: `s`
   |
26 |     let s = String::from("hello");
   |         - move occurs because `s` has type `String`, which does not implement the `Copy` trait
27 |     tx.send(s).unwrap();
   |             - value moved here
28 |     println!("{s}");
   |                ^ value borrowed here after move
   |
help: consider cloning the value if the performance cost is acceptable
   |
27 |     tx.send(s.clone()).unwrap();
   |              ++++++++
```

**"发送"这个词听起来像是一个动作**（"把东西送出去"），
你会以为它和 `println!` 一样是"用一下 `s`"。
但编译器说的是"**value moved here**" —— 发送**拿走了** `s`。

对比一下，`Arc` 版本就完全不一样：

```rust
let s = Arc::new(String::from("hello"));
tx.send(Arc::clone(&s)).unwrap();
println!("{s}");          // ✅ 编译通过
```

**同样是"发送"，一个能继续用，一个不能。** 差别在哪？

## 14.1 先把常见说法摆上桌

通常会这样概括：

- `mpsc` = multiple producer, single consumer；
- `send` 把值**移动**到 channel 里，接收端拿到所有权；
- 这就是 Rust 著名的宣传语："**通过通信来共享内存，而不是共享内存来通信**"；
- 因为所有权被转移了，发送方就不再有访问权，**数据竞争在编译期被排除**。

这些都对，但"移动"这个说法容易被当成隐喻。
本章要说明：**它不是隐喻** —— MIR 里明确记录了 `move`。
这条所有权转移与 `T: Send`、队列内部同步一起构成 channel 的安全边界。

## 14.2 编译器眼里的样子

### 14.2.1 `send_string` 的 MIR

```rust
pub fn send_string() -> usize {
    let (tx, rx) = mpsc::channel::<String>();
    let s = String::from("hello");
    let len = s.len();
    tx.send(s).unwrap();
    // println!("{s}");    // ← 取消注释会报 E0382
    drop(tx);
    let got = rx.recv().unwrap();
    got.len() + len
}
```

`.evidence/ch14-channels-lib.mir` 的关键片段：

```mir
bb3: {
    _9 = &_1;
    _19 = const false;
    _10 = move _4;                                              // ★ 把 String 从 _4 搬出来
    _8 = std::sync::mpsc::Sender::<String>::send(move _9, move _10) -> [return: bb4, unwind: bb17];
}
```

**逐行读**：

| MIR | 含义 |
|---|---|
| `_9 = &_1` | `tx` 是**借用**（`send` 的第一个参数是 `&self`） |
| `_10 = move _4` | **把 `s`（`_4`）搬到临时变量 `_10`** |
| `send(move _9, move _10)` | 两个参数都是 `move` |

★ **`_10 = move _4` 这一行是"发送即 move"的字面证据。**
这里的 `move` 是 MIR 操作数语义，不是 CPU 上名为 `move` 的指令；它表示
读取 `_4` 的值后，原位置不再被当作已初始化值使用。

搬走之后 `_4` 处于**未初始化**状态 —— 借用检查器不允许再读它。
这就是 14.0 那个 E0382 的来源。

### 14.2.2 对照：先借用原 `Arc` 做 clone，再移动新句柄

```rust
pub fn send_arc() -> usize {
    let s = Arc::new(String::from("hello"));
    tx.send(Arc::clone(&s)).unwrap();
    // ...
    got.len() + s.len()   // ← s 仍然可用
}
```

MIR：

```mir
_4 = Arc::<String>::new(move _5) -> ...;
_10 = &_4;                                                  // ★ 借用，不是 move
_7 = std::sync::mpsc::Sender::<Arc<String>>::send(move _8, move _9) -> ...;
```

**`_10 = &_4` 是为了调用 `Arc::clone` 而借用原句柄。** clone 返回的新
`Arc` 随后按值 move 进 channel；`s`（`_4`）仍留在发送端。代价是引用计数
原子加一（第 13 章的 `ldadd`）。

两条路的对照：

| | `send(String)` | `send(Arc<String>)` |
|---|---|---|
| MIR 关键动作 | 原 `String` 被 move | 借用原 `Arc` 做 clone，新 `Arc` 被 move |
| 本地还能用吗 | ❌ 不能（E0382） | ✅ 能 |
| 消息准备代价 | 不克隆消息本体 | 一次引用计数原子加 |
| 语义 | **独占**所有权转移 | **共享**所有权 |

> **这是第 13 章与本章的交汇点**：
> `Arc` 的 `ldadd` 代价换来的，正是"两边都能用"。
> 而 `send(String)` 之所以零成本，是因为**独占**意味着不需要任何同步。

### 14.2.3 为什么 `move` 是安全边界的重要一环

这是本章最深的一层。

想象 `send` 是**引用传递**（`fn send(&self, t: &T)`）。那么：

- 发送方还持有 `&T`；
- 接收方（可能在线程 B）也拿到 `&T`；
- **两个线程同时访问同一块内存** —— 数据竞争。

而按值传递（`fn send(&self, t: T)`）之后：

- 发送方**失去了** `T`（MIR 里 `_4` 未初始化）；
- 接收方**独占** `T`；
- 发送者不能再使用原值；接收者取得消息的所有权。

> **"发送即 move"不是一句宣传语，而是 channel 安全接口的重要基础。**
> 编译器先检查消息类型是否 `Send`，再用所有权转移阻止发送方继续使用原值；
> 队列内部的并发同步则由 channel 实现负责。

**所有权规则在这里**免费**变成了线程安全规则。**

### 14.2.4 ★ 实测发现：`mpsc` 内部已经是 `mpmc`

这是一个**写作时必须知道**的变化。

实测符号统计（`.evidence/ch14-channels-lib.O3.s`）：

```bash
grep -oE 'mpmc[a-zA-Z0-9_]*|sync4mpsc' .evidence/ch14-channels-lib.O3.s | sort | uniq -c | sort -rn
```

```text
    192 sync4mpmc
     27 sync4mpsc
     17 mpmc4zero5InnerEECsrtIYgyWToU_3lib
     16 mpmc5waker5WakerEECsrtIYgyWToU_3lib
```

具体的符号：

```text
__RNvMNtNtNtCs82bWklYMk3w_3std4sync4mpmc7contextNtB2_7Context3new
__RNvMsn_NtCshxvaOLs88l5_5alloc4syncINtB5_3ArcNtNtNtNtCs82bWklYMk3w_3std4sync4mpmc7context5InnerE9drop_slow...
__RNvMs0_NtNtNtCs82bWklYMk3w_3std4sync4mpmc5wakerNtB5_9SyncWaker10disconnect
```

★ **`sync4mpmc` 出现 192 次，`sync4mpsc` 只有 27 次。**
`std::sync::mpsc` 的实现**已经建立在 `sync::mpmc` 之上**
（`mpmc::zero::Inner`、`mpmc::waker::Waker`、`mpmc::context::Context`）。

> **这对写作的影响**：很多关于 `mpsc` 的旧资料（包括一些书和博客）
> 会讲"`mpsc` 内部是一个基于 `Mutex` 的链表队列"。
> **在 1.98 上已经不是这样了。**
>
> 写这一章时**不要**按旧实现描述内部结构，
> 要么按 `mpmc` 写，要么只说"具体实现是标准库内部细节"。
> 这是"证据链"方法的又一次收获：**读符号名就能发现文档过期了。**

### 14.2.5 `Send` 检查在编译期（呼应第 12 章）

```rust
pub fn send_across_thread() -> usize {
    let (tx, rx) = mpsc::channel::<Vec<u64>>();
    let h = thread::spawn(move || {
        tx.send(vec![1, 2, 3]).unwrap();
    });
    // ...
}
```

`tx` 被移进 `thread::spawn` 的闭包 —— 编译器要求闭包是 `Send`。
**汇编里没有任何"检查 `Send`"的代码**（第 12 章的结论）。

如果 payload 不是 `Send`（比如 `Rc<T>`），**编译不过**，
而不是运行时拒绝。

## 14.3 为什么必须这样设计

### 为什么 `send` 要按值，而不是按引用

因为**按引用无法表达"所有权转移"**。

按引用的 `send` 意味着：发送方保留所有权，接收方只是借看。
那么"什么时候可以安全地读"就成了一个**运行时问题**
（需要引用计数、锁、或者约定"发送后别再用"）。

Rust 把“发送后原值还能不能访问”变成了编译期所有权问题：
按值传递让发送者失去原值；channel 内部仍要用锁或原子操作同步队列。

### 为什么 `Receiver` 是 `Send` 但 `!Sync`

实测（`assert_send` / `assert_sync` 探针）：

| 类型 | `Send` | `Sync` |
|---|---|---|
| `mpsc::Sender<T>` | ✅ | ✅ |
| `mpsc::Receiver<T>` | ✅ | ❌ |

- `Receiver: Send` —— 可以**移到**另一个线程（接收端换线程）；
- `Receiver: !Sync` —— 不能**共享**（`&Receiver` 不能跨线程）。

```text
error[E0277]: `std::sync::mpsc::Receiver<u64>` cannot be shared between threads safely
  |
8 |     assert_sync::<mpsc::Receiver<u64>>();
  |                   ^^^^^^^^^^^^^^^^^^^ `std::sync::mpsc::Receiver<u64>` cannot be shared between threads safely
  |
  = help: the trait `Sync` is not implemented for `std::sync::mpsc::Receiver<u64>`
```

这与 "single consumer" 的语义**精确对应**：**只有一个消费者**。
而 `Sender: Send + Sync` 对应 "multiple producer" —— 发送端可以共享。

★ **两个 auto trait 的差异，就是它们的语义差异。**
这是第 12 章那条"`Send` / `Sync` 是类型层记账"的一个漂亮实例：
标准库**没有**写任何运行时检查来保证"只有一个消费者"，
它只是**没给 `Receiver` 实现 `Sync`**。

> 顺带：`&Receiver` 也**不能**跨线程 —— 所以"多个消费者"在 `mpsc` 里
> 是完全做不到的，而不是"能做但要小心"。

### 为什么"通过通信共享内存"比"共享内存通信"容易

因为**所有权规则本来就是为单线程设计的**。
`send` 做的事，是**把所有权规则原封不动地搬到线程边界上**：

| 单线程 | 多线程 |
|---|---|
| `let b = a;` 之后 `a` 不能用 | `tx.send(a)` 之后 `a` 不能用 |
| 借用检查器保证 | 同一个借用检查器保证 |

**没有引入任何新概念。** 这是 Rust 并发模型最优雅的地方：
线程安全不是"额外加的东西"，是**所有权规则的直接推论**。

## 14.4 反直觉的点

### 反直觉之一：`send` 是 `&self`，但拿走了 `T`

```rust
pub fn send(&self, t: T) -> Result<(), SendError<T>>
```

- 第一个参数 `&self` —— 发送端**只是借用**（可以多次 `send`）；
- 第二个参数 `t: T` —— **按值**，所有权转移。

**"借用发送端，拿走消息体"** —— 这个组合就是全部。
`&self` 让一个 `Sender` 句柄可以重复发送；克隆句柄后可有多个生产者，
`t: T` 让消息体独占。

> 很多人第一次看到 E0382 会以为"`send` 借用了 `s`"。
> 恰恰相反：它**拿走了** `s`，但**只借用了** `tx`。

### 反直觉之二：`send` 的返回值里有 `T`

```rust
pub fn send(&self, t: T) -> Result<(), SendError<T>>
```

**发送失败时，值会被还回来** —— 在 `SendError<T>` 里。

这看起来是个小细节，但它揭示了一个设计原则：
**失败路径上的所有权也必须明确**。
如果 `send` 失败就丢弃 `T`，那么"发送失败"就意味着**数据静默丢失**。

对照 `MIR` 里的失败路径（`.mir` 第 62 行附近）：

```mir
_10 = move _4;          // 搬出来
...
drop(_4) -> [return: bb14, unwind terminate(cleanup)];   // ← 失败路径上才 drop
```

**成功路径上 `_4` 不会被 drop**（所有权已经交出去了）；
**失败路径上才 drop**。这是借用检查器算出来的，不是手写的。

### 反直觉之三：`Arc` 版本"更贵"，但更常用

| | `send(String)` | `send(Arc<String>)` |
|---|---|---|
| 汇编里的代价 | 零（一条 move） | 一次原子加 |
| 本地还能用吗 | ❌ | ✅ |
| 什么时候用 | 数据是"一次性"的 | 数据要**多方**读 |

**"零成本"不是选择标准的全部。**
`send(String)` 的零成本来自"独占"—— 而独占意味着**数据只能有一个读者**。
很多时候你需要的是"多个读者"，那就必须付 `Arc` 的原子操作代价。

> **这是一条通用原则**：
> 独占换性能，共享换灵活性。**Rust 的类型系统让你必须显式选择。**

### 反直觉之四：`mpsc` 内部已经不是 `mpsc` 了

实测发现 `std::sync::mpsc` 的实现**建立在 `sync::mpmc` 之上**
（`sync4mpmc` 在汇编里出现 **192** 次，`sync4mpsc` 只有 27 次）。

**这意味着很多关于 `mpsc` 的旧资料是过期的。**
具体来说，常见的"`mpsc` 内部是一个基于 `Mutex` 的链表"这种描述
在 1.98 上已经不对了。

> **方法论的收获**：这条结论不是从文档里读来的，
> 是**数符号名数出来的**。
> `grep -oE 'mpmc[a-z0-9_]*' ... | sort | uniq -c | sort -rn`
> 这个命令值得记住 —— 它能在不读源码的情况下
> 快速判断"某个 std 类型的内部实现换没换"。

## 14.5 亲手验证

```bash
tools/evidence.sh ch14-channels
scripts/verify-all.sh ch14

# ★ "发送即 move"的字面证据
grep -n '_10 = move _4' .evidence/ch14-channels-lib.mir
grep -n 'Sender::<String>::send(move _9, move _10)' .evidence/ch14-channels-lib.mir

# Arc 版本是借用
grep -n '_10 = &_4' .evidence/ch14-channels-lib.mir

# ★ mpsc 内部已经是 mpmc
grep -oE 'sync4mpmc|sync4mpsc' .evidence/ch14-channels-lib.O3.s | sort | uniq -c
```

**怎么算验证成功**：

1. MIR 里有 `_10 = move _4;` —— **发送前先把局部变量搬出来**；
2. `send(move _9, move _10)` —— 两个参数都是 `move`；
3. `Arc` 版本对应的是 `_10 = &_4;`（**借用**，不是 move）；
4. `fail/use_after_send.rs` 报 **E0382**（`value moved here`）；
5. `grep -c 'sync4mpmc'` 远大于 `grep -c 'sync4mpsc'`
   —— `mpsc` 内部已经是 `mpmc`。

```bash
scripts/verify-all.sh ch14      # 4 条断言
```

## 14.6 与 unsafe 的关系

这一章**几乎没有 `unsafe`** —— 这正是它的价值所在。

`send` 的所有权转移是**完全安全的代码**：
没有 `unsafe`、没有运行时检查、没有原子操作（对 `String` 而言）。
**并发安全是靠类型系统免费得到的。**

★ 但反过来看，channel 的**内部实现**全是 `unsafe`：

- 共享的队列需要 `UnsafeCell`（内部可变性）；
- 多生产者需要原子操作；
- waker 的注册/唤醒需要仔细的生命周期管理。

**这就是"安全抽象 + 不安全实现"的典型结构**：
接口层完全安全（`send` / `recv` 都是 safe fn），
实现层用经过论证的底层操作实现并发队列。

★ 而接口层之所以能是安全的，靠的正是本章讲的那条 `move`：
**所有权转移把"两个线程同时访问"这件事从根上排除了**，
所以内部实现不需要处理这种情况 —— 它只需要处理"队列本身"的并发。

> **第 24 章会展开这个模式**：`unsafe` 的正确用法是
> **把不安全收敛到一小块，让接口层的安全性有证明**。
> `mpsc` 是很好的教材：底层不安全操作被收敛在队列实现里，
> 而"发送即 move"这条类型层的规则，承担了很大一部分安全性。

## 14.7 小结

- **`send` 的签名是 `fn send(&self, t: T)`**：
  借用发送端，**拿走**消息体。这个组合就是"发送即 move"的全部。
- **MIR 里明确写着 `move`**：`_10 = move _4`。这是 MIR 的所有权语义，
  不要把它误解成某条同名机器指令。
- **这条 `move` 是 channel 安全边界的重要一环**：发送方失去原值，
  接收方取得所有权；`T: Send` 与队列内部同步共同完成线程安全保证。
- **`Arc` 版本是借用**（`_10 = &_4`），所以本地还能用 ——
  代价是一次原子加（第 13 章的 `ldadd`）。
  **独占换性能，共享换灵活性。**
- **失败路径上所有权也要明确**：`SendError<T>` 把值还回来，
  否则"发送失败"就意味着数据静默丢失。
- **★ `mpsc` 内部已经是 `mpmc`**：实测 `sync4mpmc` 出现 192 次、
  `sync4mpsc` 只有 27 次。**旧资料里"基于 `Mutex` 的链表"的描述已经过期。**
- **调用方不用写 `unsafe`**，但 channel 操作并非没有运行时成本；
  队列内部的同步和底层不安全操作被封装在安全 API 后面。

下一章继续深入"共享可变状态"：当 `Arc` 和 `Mutex` 不够用时，
怎么用**类型**表达"谁能改、谁能读"。
