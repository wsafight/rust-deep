# 27. `Drop`、drop flags 与 panic safety

> 一句话：所有权决定谁负责释放资源，`Drop` 决定释放动作；而 drop flags 和
> cleanup control flow 保证“只析构已经初始化、且仍由这里拥有的值”。

## 先把规则认清

Rust 的资源管理通常概括成 RAII，但这四个字母掩盖了几个不同问题：

- 一个值在正常控制流的哪个位置析构？
- 值被条件移动后，编译器怎么知道还要不要析构？
- panic 展开时，已经构造好的局部变量由谁清理？
- 为什么实现了 `Drop` 的类型不能直接移出字段？

这些问题不只影响文件和锁。连接、事务、临时目录、线程、FFI handle，以及
异步 future 被取消时的清理，最后都会落到同一套析构语义上。

### 放到业务里：更新两个字段时发生 panic

假设缓存更新要依次替换索引和数据。如果替换索引后，构造数据时 panic：

```rust,ignore
self.index = build_index(input);
self.data = build_data(input); // 这里 panic
```

Rust 会释放已经初始化的临时值，但不会自动把 `self.index` 恢复成旧值。
因此要区分：

- **资源安全**：内存、锁、文件句柄不会泄漏；
- **逻辑 panic safety**：对象仍满足业务不变式。

RAII 主要解决前者。后者需要“先在临时对象中完成，再一次提交”的设计。

## 27.0 一个会让你卡住的例子

```rust,compile_fail,E0509
struct Envelope { payload: String }

impl Drop for Envelope {
    fn drop(&mut self) {}
}

fn take_payload(envelope: Envelope) -> String {
    envelope.payload
}
```

错误是 E0509：不能从实现了 `Drop` 的类型里移出字段。

原因不是编译器不知道 `String` 怎么移动，而是移动之后仍要调用
`Envelope::drop(&mut self)`。此时析构器会收到一个字段已经不存在的
`Envelope`，但它的签名和实现都没有办法表达“payload 已搬走”。

标准修法是把需要提前取出的字段写成 `Option<T>`：

```rust
struct Envelope { payload: Option<String> }

impl Envelope {
    fn take_payload(&mut self) -> Option<String> {
        self.payload.take()
    }
}
```

对象始终有效，只是字段从 `Some` 变成 `None`。第 17 章在线程池的 `Drop`
实现里也是这样先 `take()` sender，再等待 worker。

## 27.1 两套容易记反的顺序

局部变量和结构体字段的析构顺序不同：

| 对象 | 析构顺序 |
|---|---|
| 同一作用域的局部变量 | 声明的逆序 |
| tuple 字段 | 从第 0 个到最后一个 |
| struct 字段 | 字段的声明顺序 |
| enum | 当前活跃 variant 的字段声明顺序 |

本章的运行期测试没有猜测实现细节，而是让每个析构器记录自己的编号：

```rust,ignore
{{#include ../../examples/ch27-drop/src/lib.rs:local_drop_order}}
```

完整实现位于 `examples/ch27-drop/src/lib.rs`，验证结果是：

```text
local_drop_order() == [2, 1]
field_drop_order() == [1, 2]
```

字段顺序是 API 设计的一部分。锁 guard、FFI handle 或依赖另一个字段的资源，
不应靠“看起来像栈”来猜释放顺序。

## 27.2 编译器眼里的 drop flag

看一个值只在某条分支被移动的函数：

```rust,ignore
pub fn conditional_move(take: bool) -> usize {
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let value = Counted::new(7, &drops);
        if take {
            consume(value);
        }
    }
    drops.load(Ordering::Relaxed)
}
```

若走 `take == true`，`consume(value)` 已经接管所有权，函数结尾不能再 drop；
若走另一条分支，函数仍然拥有 `value`，必须 drop。

MIR 为此维护一个布尔状态，并在正常出口和 cleanup 路径检查它。这个状态通常
称为 **drop flag**：

```text
构造 value     -> flag = true
move value     -> flag = false
离开作用域     -> if flag { drop(value) }
```

这解释了几个常见现象：

- 移动本身不会立即调用析构器；
- 被移动的旧位置不会再次析构；
- 部分初始化失败时，只清理已经初始化的字段；
- `unsafe` 手写初始化代码必须自己维护同样的不变式。

最后一条会在第 29、30 章直接决定 `MaybeUninit` 和 `ArrayVec` 是否 sound。

## 27.3 panic 路径不是“突然消失”

默认的 `panic=unwind` 下，MIR 的函数调用有正常边和 unwind 边：

```mir
call(...) -> [return: bb_normal, unwind: bb_cleanup];
```

本章的 `unwind_cleanup` 在栈上放一个计数 guard，然后选择是否 panic。实测
正常返回和 panic 展开都恰好析构一次；MIR 的 cleanup block 在析构后执行
`resume`，继续向上传播 panic。

因此 `MutexGuard` 能在 panic 时解锁，`File` 能关闭句柄。但有三个边界：

1. `panic=abort` 不展开栈；
2. 进程被杀、断电或 `abort` 时不会可靠运行析构器；
3. 析构器能释放资源，不能猜测业务状态如何回滚。

所以不要在 `Drop` 里承担“必须成功写盘”或“必须通知远端”这类承诺。

## 27.4 drop check：析构时借用必须仍有效

`Drop::drop` 接收 `&mut self`，因此它原则上可以读取所有字段。若字段里保存了
引用，被借用的数据必须活到析构发生之后：

```rust,compile_fail,E0597
struct Inspector<'a>(&'a str);

impl Drop for Inspector<'_> {
    fn drop(&mut self) { println!("{}", self.0); }
}

fn rejected() {
    let inspector;
    let value = String::from("short lived");
    inspector = Inspector(&value);
}
```

局部变量逆序析构：`value` 后声明，所以会先析构；但 `inspector` 的析构器随后
还可能读取它。drop check 因此在编译期报 E0597。

这也是第 25 章 `PhantomData` 会影响 drop check 的原因：裸指针字段本身不会
告诉编译器“我在逻辑上拥有或借用了 T”，需要类型显式补回这个关系。

## 27.5 `forget` 为什么是安全的

`std::mem::forget(value)` 消耗值但不运行它的析构器，而且它是安全函数。
这不是漏洞，而是一条重要边界：Rust 的安全性不能依赖析构器一定执行。

泄漏内存或句柄通常是资源 bug，却不自动构成内存不安全。安全抽象的 soundness
必须在调用者永久忘记某个值时仍然成立。反过来说，若忘掉一个 guard 就能制造
悬垂引用或越界访问，这个“安全”API 本身就是 unsound 的。

## 27.6 亲手验证

```bash
cargo test -p ch27-drop
tools/evidence.sh ch27-drop

# 条件移动与 cleanup control flow
sed -n '/fn conditional_move/,/^}/p' .evidence/ch27-drop-lib.mir
sed -n '/fn unwind_cleanup/,/^}/p' .evidence/ch27-drop-lib.mir

# 两个反例与自动断言
scripts/verify-evidence.sh ch27
```

应观察到：

1. 局部变量顺序为 `[2, 1]`，字段顺序为 `[1, 2]`；
2. 条件移动的两条路径都只析构一次；
3. panic 被 `catch_unwind` 捕获后，guard 已经析构；
4. 移出 `Drop` 类型的字段报 E0509；
5. 析构时可能悬垂的引用报 E0597。

## 27.7 小结

- 所有权移动和析构是同一件事的两面：责任被移动后，旧位置不再析构；
- drop flag 让条件移动、部分初始化和 unwind 都能做到“只析构有效值”；
- 局部变量逆序析构，结构体字段按声明顺序析构；
- RAII 保证资源 cleanup，不自动保证业务事务回滚；
- `Drop` 类型不能直接移出字段，通常用 `Option::take` 表达状态变化；
- soundness 不能依赖析构器一定运行，因为 `mem::forget` 本身就是安全的。
