# 第 14 章：消息传递 —— 发送即 move，编译期切断别名 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch14-channels
scripts/verify-all.sh ch14       # 4 条断言（PASS=6）
```

## 关键结论与断言（4 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | `send` 按值拿走所有权（MIR 里是 move） | `Sender::<String>::send(move _9, move _10)` |
| 2 | ★ 发送前先把局部变量 move 出来 | `.mir` 里 `_10 = move _4` |
| 3 | `Arc` 版本：`Arc::new` 之后本地仍然可用 | `.mir` 里 `Arc::<String>::new(move _5)` |
| 4 | 发送之后本地再用它 → E0382 | `fail/use_after_send.rs` |

## ★ 核心证据：MIR 里"发送即 move"是**字面意义**的

`send_string` 的关键 MIR（`.evidence/ch14-channels-lib.mir`）：

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
| `_9 = &_1` | `tx` 是**借用**（`send` 第一个参数是 `&self`） |
| `_10 = move _4` | **把 `s`（`_4`）搬到临时变量 `_10`** |
| `send(move _9, move _10)` | 两个参数都是 `move` |

★ **`_10 = move _4` 就是"发送即 move"的字面证据。**
不是"语义上等价于移动"，而是**真的有一条 `move`**。

搬走之后 `_4` 处于未初始化状态 —— 借用检查器不允许再读它。

失败路径也值得看（同一份 `.mir` 第 62 行附近）：

```mir
drop(_4) -> [return: bb14, unwind terminate(cleanup)];      // ← 失败路径上才 drop
```

**成功路径上 `_4` 不会被 drop**（所有权已经交出去了）。
这是借用检查器算出来的，不是手写的。

## ★ 对照：`Arc` 版本是**借用**，不是 move

`send_arc` 的 MIR：

```mir
_4 = Arc::<String>::new(move _5) -> ...;
_10 = &_4;                                                  // ★ 借用，不是 move
_7 = std::sync::mpsc::Sender::<Arc<String>>::send(move _8, move _9) -> ...;
```

| | `send(String)` | `send(Arc<String>)` |
|---|---|---|
| MIR | `_10 = move _4` | `_10 = &_4` |
| 本地还能用吗 | ❌ 不能（E0382） | ✅ 能 |
| 代价 | **零**（所有权转移） | 一次原子加（`ldadd`，见第 13 章） |
| 语义 | **独占**所有权转移 | **共享**所有权 |

→ **第 13 章与本章的交汇点**：`Arc` 的 `ldadd` 代价换来的，
正是"两边都能用"。

## 反例：`fail/use_after_send.rs`

```text
error[E0382]: borrow of moved value: `s`
  --> examples/ch14-channels/fail/use_after_send.rs:28:16
   |
26 |     let s = String::from("hello");
   |         - move occurs because `s` has type `String`, which does not implement the `Copy` trait
27 |     tx.send(s).unwrap();
   |             - value moved here
28 |     println!("{s}");        // ← E0382
   |                ^ value borrowed here after move
   |
help: consider cloning the value if the performance cost is acceptable
   |
27 |     tx.send(s.clone()).unwrap();
   |              ++++++++
```

★ 注意编译器的建议是 `s.clone()` —— 但那**不是**等价替换：
`clone` 让发送方和接收方各持一份，**语义完全不同**（从"转移"变成"复制"）。

## ★★ 实测发现：`mpsc` 内部已经是 `mpmc`

这是一个**写作时必须知道**的变化。

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

> **对写作的影响**：很多关于 `mpsc` 的旧资料会讲
> "内部是一个基于 `Mutex` 的链表队列"。**在 1.98 上已经不是这样了。**
> 写正文时不要按旧实现描述内部结构。
>
> **方法论的收获**：这条结论不是从文档里读来的，
> 是**数符号名数出来的**。
> `grep -oE '...' | sort | uniq -c | sort -rn` 这个套路
> 能在不读源码的情况下快速判断"某个 std 类型的内部实现换没换"。

## `Sender` / `Receiver` 的 auto trait（实测）

```rust
fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}
```

| 类型 | `Send` | `Sync` |
|---|---|---|
| `mpsc::Sender<T>` | ✅ | ✅ |
| `mpsc::Receiver<T>` | ✅ | ❌ |

```text
error[E0277]: `std::sync::mpsc::Receiver<u64>` cannot be shared between threads safely
  |
8 |     assert_sync::<mpsc::Receiver<u64>>();
  |                   ^^^^^^^^^^^^^^^^^^^ `std::sync::mpsc::Receiver<u64>` cannot be shared between threads safely
  |
  = help: the trait `Sync` is not implemented for `std::sync::mpsc::Receiver<u64>`
```

★ **两个 auto trait 的差异就是它们的语义差异**：
`Sender: Send + Sync` ↔ "multiple producer"；
`Receiver: Send + !Sync` ↔ "single consumer"。

标准库**没有**写任何运行时检查来保证"只有一个消费者" ——
它只是**没给 `Receiver` 实现 `Sync`**。

## `Send` 检查在编译期（呼应第 12 章）

`send_across_thread` 把 `tx` 移进 `thread::spawn` 的闭包 ——
编译器要求闭包是 `Send`。**汇编里没有任何"检查 `Send`"的代码**。

如果 payload 不是 `Send`（比如 `Rc<T>`），**编译不过**，
而不是运行时拒绝。

## 交叉验证（可选）

```bash
grep -n '_10 = move _4' .evidence/ch14-channels-lib.mir
grep -n '_10 = &_4'     .evidence/ch14-channels-lib.mir
grep -oE 'sync4mpmc|sync4mpsc' .evidence/ch14-channels-lib.O3.s | sort | uniq -c
```

## 待办

- [ ] 补 `sync_channel`（有界）与 `Receiver::try_recv` 的对照
- [ ] 第 15 章（共享可变状态的所有权设计）需要自己的 example
