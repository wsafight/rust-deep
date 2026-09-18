# 21. `async fn in trait` 的现状

> 一句话：**`async fn` 在 trait 里隐藏了返回类型** ——
> 这既是它能用起来的原因，也是它**不能表达 `Send`、不能 `dyn`** 的原因。
> 三个需求（`dyn` / `Send` / 零分配）在 1.98 上**最多同时满足两个**。

第 18–20 章把 `async` 从里到外拆了一遍：状态机、`Pin`、生命周期、`Send` 传染。
这一章换一个方向 —— **当 `async fn` 出现在 trait 里**，
前面那些机制会撞上什么。

★ **本章的结论演进最快**，请以本章顶部的"最后验证"标注为准。

## 21.0 一个会让你卡住的例子

你想定义一个异步的存储抽象：

```rust
pub trait Store {
    async fn get(&self, k: u64) -> u64;
}
```

**这段编译过了**（1.98 上 `async fn in trait` 已稳定）。但编译器给了个警告：

```text
warning: use of `async fn` in public traits is discouraged as auto trait
         bounds cannot be specified
  = note: you can suppress this lint if you plan to use the trait only in your
          own code, or do not care about auto traits like `Send` on the Future
  = note: `#[warn(async_fn_in_trait)]` on by default
```

你决定忽略它（`#![allow(async_fn_in_trait)]`），然后在服务端代码里：

```rust
tokio::spawn(async move { store.get(1).await });
```

```text
error: future cannot be sent between threads safely
note: the trait bound `impl Future<Output = u64>: Send` is not satisfied
```

**`tokio::spawn` 要求 `F: Send + 'static`，而 trait 没有承诺 `Send`。**

再看第二个需求 —— 你想 `Box<dyn Store>`：

```rust
pub fn make() -> Box<dyn Store> { Box::new(Mem) }
```

```text
error[E0038]: the trait `Store` is not dyn compatible
  = note: for a trait to be dyn compatible it needs to allow building a vtable
```

**为什么？** 这两条错误指向同一个根源，而那条 lint 已经把答案说出来了：
**"auto trait bounds cannot be specified"**。

## 21.1 表层解释（官方书会怎么讲）

官方书会说：

- `async fn in trait` 在 Rust 1.75 稳定（**AFIT**）；
- 它等价于返回 `impl Future` 的方法（**RPITIT**）；
- 缺点是不能 `dyn`、不能表达 `Send`，需要 `trait-variant` 之类的宏来补；
- 更完整的方案（`return_type_notation`、`async fn in dyn trait`）还在路上。

这些都对。但"不能 `dyn`、不能表达 `Send`"是**症状**，
本章要给出**病因** —— 而且病因只有一个。

## 21.2 编译器眼里的样子

### 21.2.1 病因：返回类型是**不透明的**

```rust
pub trait Store {
    async fn get(&self, k: u64) -> u64;      // 返回什么类型？
}
```

答案：**一个对每个实现都不同的、匿名的状态机类型**（第 18 章）。

对 `Mem` 的实现，它大致是 `{async fn body of Mem::get}`；
对另一个实现，它是**另一个**类型。

这就是全部问题的来源：

| 需求 | 为什么不满足 |
|---|---|
| `dyn Store` | vtable 要**固定布局**，而每个实现的返回类型**大小不同** |
| `Send` bound | trait 只承诺"是 `Future`"，**没承诺是 `Send`** |
| 递归 | 状态机大小无穷（第 18 章 E0733） |

**三个症状，一个病因。**

### 21.2.2 但它真的能跑，而且零分配

先看好的一面。MIR 里能看到泛型调用是**单态化**的：

```mir
fn use_store_mem::{closure#0}(...) -> Poll<u64> {
    coroutine layout {
        field _s0: {async fn body of use_store<Mem>()};   // ★ 装着具体类型的状态机
        ...
    }
}
```

★ **注意 `use_store<Mem>`** —— 泛型参数被替换成了具体类型。
这就是单态化（第 7 章）：每个 `S` 生成一份独立的代码。

汇编里的证据（`tools/evidence.sh ch21-afit`）：

```asm
_use_store_mem:
	strb	wzr, [x8, #40]      ; 只写判别式
	ret
```

**没有 `Box`、没有 `alloc`。** AFIT 的泛型路径是**零分配**的 ——
状态机直接建在调用者的栈/结构里。

> ⚠️ 顺带一条：**泛型函数不能 `#[unsafe(no_mangle)]`** ——
> rustc 会警告 `functions generic over types or consts must be mangled`。
> 因为它会被单态化成多份。**这个警告本身就是"AFIT 走单态化"的旁证。**

### 21.2.3 `dyn` 路线：`Box<dyn Future>`

要 `dyn`，必须把返回类型**统一**成"胖指针"：

```rust
pub trait StoreDyn {
    fn get(&self, k: u64) -> Box<dyn Future<Output = u64> + Send + '_>;
}
```

代价：**一次堆分配**（每次调用）。

★ 而且**不能直接 `.await`**（`fail/box_dyn_future_not_awaitable.rs`）：

```rust
pub async fn use_dyn(s: &dyn StoreDyn) -> u64 {
    StoreDyn::get(s, 1).await      // ← E0277
}
```

```text
error[E0277]: `dyn Future<Output = u64>` cannot be unpinned
  = note: required for `Box<dyn Future<Output = u64>>` to implement `Unpin`
```

**这是第 19 章的知识在新地方冒出来**：
`.await` 的 blanket impl 要求 `F: Future + Unpin`，
而 **trait object 默认 `!Unpin`**（vtable 里没有 `Unpin` 的信息）。

修法是 `Box::into_pin`：

```rust
let mut f = Box::into_pin(StoreDyn::get(s, 1));
std::future::poll_fn(|cx| f.as_mut().poll(cx)).await
```

★ **注意这个错误恰好出现在最需要 `dyn` 的地方** ——
于是"用 `dyn` 解决 AFIT 的 dyn 问题"这条路上，**必然要过 `Pin` 这一关**。

汇编里能看到胖指针被存进状态机：

```asm
_use_store_dyn:
	stp	x0, x1, [x8]        ; ★ (data, vtable) 一起存
	strb	wzr, [x8, #40]
	ret
```

`stp x0, x1` —— 两个寄存器一起存，这就是 `Box<dyn Future>` 的 16 字节。

### 21.2.4 解法：RPITIT 手写 `+ Send`

```rust
pub trait StoreSend {
    fn get(&self, k: u64) -> impl Future<Output = u64> + Send;
}
```

**这次 `Send` 写进签名了**，调用点就能过：

```rust
pub fn spawn_ok(s: &Mem) {
    assert_send(StoreSend::get(s, 1));   // ✅
}
```

代价：

1. **不能再简写 `async fn`**（`async fn` 没法写 `+ Send`）；
2. **每个实现都得手写 `impl Future` 包装**。

★ 而它带来一个**好处**（21.4 展开）：`Send` 的检查点**前移**了。

### 21.2.5 一张对照表

| 写法 | 能 `dyn` | 能表达 `Send` | 分配 |
|---|---|---|---|
| `async fn` in trait | ❌ E0038 | ❌ | 无 |
| RPITIT `-> impl Future + Send` | ❌ | ✅ | 无 |
| `-> Box<dyn Future + Send>` | ✅ | ✅ | **一次堆分配** |

★ **没有一列全是 ✅ 的行。** 这就是 1.98 上 AFIT 的现状。

## 21.3 为什么必须这样设计

### 为什么 `async fn` 的返回类型必须是不透明的

因为**它的类型没法写出来** —— 那是编译器生成的匿名状态机
（第 18 章：`{async fn body of ...}`）。

如果要让它透明，得引入一个"能指称匿名类型"的语法 ——
那正是 `impl Trait` 在做的事。所以 `async fn in trait` 在语义上
**就是** `-> impl Future`（RPITIT）。

**不透明不是缺陷，是"类型写不出来"的必然结果。**
而 `dyn` 和 `Send` 这两件事，恰恰都要求**返回类型是已知的**：

- `dyn` 要 vtable，vtable 要**固定布局**；
- `Send` bound 要检查具体类型。

★ 所以这两条限制**不是"还没实现"** ——
`dyn` 那条是"按现在的 vtable 模型无法实现"（21.4 展开）。

### 为什么 `async fn` 在 trait 里不推荐（那条 lint 的由来）

因为**你无法表达"返回的 future 是 `Send`"**，而这是**绝大多数真实场景的要求**
（`tokio::spawn` 要它、`futures::join!` 在多线程 executor 上要它）。

lint 的措辞非常精确：

```text
use of `async fn` in public traits is discouraged as auto trait bounds
cannot be specified
```

**"auto trait bounds cannot be specified"** —— `Send` / `Sync` 是 auto trait
（第 12 章），而 `async fn` 的简写形式**没有地方写它们**。

★ 注意 lint 说的是 **public traits**。如果你只在**自己的 crate 内部**用，
而且不在乎 `Send`，那 `async fn` 完全够用 ——
**`#![allow(async_fn_in_trait)]` 是合理的**。

### 为什么 `Box<dyn Future>` 是"最后手段"

因为它把"零成本"还回去了：

| | AFIT（泛型） | `Box<dyn Future>` |
|---|---|---|
| 分配 | 无 | **每次调用一次** |
| 分发 | 单态化（静态） | vtable（间接） |
| 代码大小 | 每个类型一份 | 一份 |

**这正是第 7 章的 `dyn vs 泛型` 在异步里的重演**：
`dyn` 换来了运行时的灵活性，代价是分配 + 间接调用。

## 21.4 反直觉的点

### 反直觉之一：`Send` 的问题**不是**"编译器不够聪明"

初学时的反应是："编译器知道 `Mem::get` 的 future 是 `Send` 啊，为什么不放行？"

**因为 trait 的承诺是抽象的**：`spawn_it<S: Store>` 要对**所有** `S` 成立，
而 `Store` 没有承诺 `Send`。某个实现完全可以捕获 `Rc`（21.4 反直觉之二）。

★ 这不是"保守"，是**唯一正确的做法**。
要放行，必须让 **trait 自己承诺** `Send` —— 那就是 RPITIT 的写法。

### 反直觉之二：RPITIT 的 `+ Send` 让报错位置**变好了**

看 `fail/rpitit_send_at_impl.rs`：

```rust
impl StoreSend for Bad {
    fn get(&self, k: u64) -> impl Future<Output = u64> + Send {
        async move {
            let r = std::rc::Rc::new(k);        // ← Rc: !Send
            std::future::ready(()).await;        // ← r 跨过 await
            *r
        }
    }
}
```

错误**报在 `impl` 里那几行**，而不是任何调用点。

★ **这是好事**：第 20 章讲的那个"`Send` 沿 `.await` 传染、
报错位置离原因很远"的问题，在这里**被提前到了原因所在的地方**。

对比两种写法：

| 写法 | 错误报在哪 | 好不好 |
|---|---|---|
| `async fn`（trait 没承诺 `Send`） | **调用点** | ❌ 离原因远 |
| RPITIT `+ Send`（签名承诺了） | **实现处** | ✅ 就在原因上 |

**代价是：实现者必须自己处理这个约束，而 `async fn` 简写给不了这个选择。**

### 反直觉之三：AFIT 的泛型路径是**零分配**的

直觉上"trait 方法 + 异步"听起来就要装箱。**实测不是**：

```asm
_use_store_mem:
	strb	wzr, [x8, #40]
	ret
```

**没有 `Box`、没有 `alloc`。** 状态机直接建在调用者提供的地方。

★ 所以"异步 trait 有开销"这个印象是错的 ——
**开销取决于你选哪条路**，而不是"用了 trait"。

### 反直觉之四：`dyn` 和 AFIT 的冲突是**根本性的**

不是"还没实现"，是"按现在的 vtable 模型无法实现"。

vtable 是一张**固定布局**的函数指针表（第 7 章：
前 3 个 slot 是 `drop` / `size` / `align`，方法从偏移 24 起）。
`async fn` 的返回类型**每个实现都不同** —— 表就定不下大小。

★ 要支持，得改 vtable 的模型本身（比如加一层间接）。
所以这条路**比 `Send` 那条长得多** ——
**`Send` 已经有 RPITIT 解法了，`dyn` 还没有。**

### 反直觉之五：`Box<dyn Future>` 不能直接 `.await`

这个错误让人意外 —— 明明 `.await` 就是给 `Future` 用的。

**因为 `.await` 的 blanket impl 要求 `F: Future + Unpin`**（第 19 章），
而 trait object 默认 `!Unpin`。

★ 又一次印证第 19 章那句话：
**`Pin` 不是一个"高级话题"，它会在你最意想不到的地方出现** ——
比如这里，在"想给异步 trait 加动态分发"这个看起来跟 `Pin` 无关的需求上。

修法只有两条：`Box::into_pin`（把 `Box` 变成 `Pin<Box>`），
或者 `Box::pin`。

## 21.5 亲手验证

```bash
tools/evidence.sh ch21-afit
scripts/verify-all.sh ch21      # 8 条断言

# ★ 泛型路径是单态化 + 零分配
grep -n 'field _s0: {async fn body of use_store<Mem>()}' .evidence/ch21-afit-lib.mir
awk '/^_use_store_mem:/{on=1} on{print} on&&/cfi_endproc/{exit}' .evidence/ch21-afit-lib.O3.s

# ★ dyn 路线：胖指针被存进状态机
awk '/^_use_store_dyn:/{on=1} on{print} on&&/cfi_endproc/{exit}' .evidence/ch21-afit-lib.O3.s

# ★ 四个反例
for f in afit_not_dyn afit_not_send box_dyn_future_not_awaitable rpitit_send_at_impl; do
  echo "--- $f ---"
  rustc --edition 2024 --crate-type=lib examples/ch21-afit/fail/$f.rs 2>&1 | head -3
done
```

**怎么算验证成功**：

1. MIR 里 `use_store_mem` 的布局字段是
   `{async fn body of use_store<Mem>()}` —— **泛型被替换成具体类型**（单态化）；
2. `_use_store_mem` 的汇编**只有 `strb` + `ret`**，没有 `alloc` —— **零分配**；
3. `_use_store_dyn` 的汇编里有 `stp x0, x1, [x8]` ——
   **`(data, vtable)` 胖指针**；
4. 四个反例分别报：
   **E0038**（不是 dyn compatible）、
   `future cannot be sent between threads safely`（×2）、
   **E0277** `cannot be unpinned`。

## 21.6 与 unsafe 的关系

本章**没有 `unsafe`** —— 但它是全书**最直接体现"`unsafe` 的替代品是设计"**
的一章。

理由：AFIT 的三个限制（`dyn` / `Send` / 分配），
**没有一个是 `unsafe` 能解决的**。

- 想给"不能 `dyn` 的 trait"强加 `dyn`？**做不到** ——
  vtable 建不出来（E0038 是硬性检查）。
- 想 `unsafe impl Send for MyFuture` 来绕开 `Send` 限制？
  **第 20 章已经论证过**：`Send` 不是一个"要不要"的开关，
  它是一个"是不是"的事实。`unsafe impl` 是在替编译器撒谎，
  而谎言会以"锁在错误的线程上释放"的形式兑现。
- 想用 `transmute` 把状态机转成 `Box<dyn Future>`？
  **状态机的大小对每个实现都不同**，转不了。

★ **所以本章的"解法"全部是设计层面的**：
换签名（RPITIT 加 `+ Send`）、换返回类型（`Box<dyn Future>`）、
或者接受限制（只在 crate 内部用 `async fn`）。

> **这是第 26 章那条判据的一个补充：**
> 有些问题 `unsafe` 解决不了，**只能靠改设计**。
> 而"想用 `unsafe` 绕过类型系统的限制"往往是**找错了工具** ——
> 因为那些限制通常不是"检查太严"，而是"**这件事本身表达不出来**"。

## 21.7 小结

- **`async fn` 在 trait 里隐藏了返回类型** ——
  这既是它能用起来的原因（类型写不出来），
  也是它不能 `dyn`、不能表达 `Send` 的原因。
- **三个症状一个病因**：返回类型是**每个实现都不同的匿名状态机**。
  `dyn` 要固定布局的 vtable、`Send` 要检查具体类型 —— 两者都要求类型已知。
- **AFIT 的泛型路径是零分配 + 单态化**：
  `_use_store_mem` 只有 `strb` + `ret`，MIR 里字段是
  `{async fn body of use_store<Mem>()}`。**"异步 trait 有开销"是错的。**
- **要 `dyn` 必须回到 `Box<dyn Future>`**，代价是每次调用一次堆分配；
  而且它**不能直接 `.await`**（`!Unpin`）—— 要用 `Box::into_pin`。
- **要 `Send` 就用 RPITIT 手写 `+ Send`**：
  代价是不能简写 `async fn`，**收益是 `Send` 的检查点前移到实现处**。
- **没有一列全是 ✅ 的方案**：`dyn` / `Send` / 零分配，**最多同时满足两个**。
- **这些限制 `unsafe` 解决不了** ——
  它们不是"检查太严"，是"**这件事本身表达不出来**"。
  这一章的全部解法都在设计层面。

下一章换到工程视角：**tokio**。
我们会看到本章那个 `Send + 'static` 的要求，在真实项目里是什么形态。
