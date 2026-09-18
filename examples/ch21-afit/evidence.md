# 第 21 章：`async fn in trait` 的现状 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
>
> ⚠️ **本章结论演进最快**（AFIT / RPITIT 仍在推进）。
> 以本文件顶部的"最后验证"标注为准。

## 复现命令

```bash
tools/evidence.sh ch21-afit
scripts/verify-all.sh ch21      # 8 条断言
```

## 关键结论与断言（8 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | ★ AFIT 走单态化 | `.mir` 里 `field _s0: {async fn body of use_store<Mem>()}` |
| 2 | trait 方法调用点的返回类型仍是状态机 | `.mir` 里 `fn use_store(_1: &S) -> {async fn body of use_store<S>()}` |
| 3 | ★ AFIT 零分配 | `use_store_mem` 函数体内**无** `alloc` |
| 4 | dyn 路线：胖指针存进状态机 | `use_store_dyn` 函数体内 `stp x0, x1, [x8]` |
| 5 | ★ `async fn` 的 trait 不是 dyn compatible | `fail/afit_not_dyn.rs` → **E0038** |
| 6 | ★ AFIT 表达不了 `Send` | `fail/afit_not_send.rs` → `future cannot be sent between threads safely` |
| 7 | ★ `Box<dyn Future>` 不能直接 await | `fail/box_dyn_future_not_awaitable.rs` → `cannot be unpinned` |
| 8 | ★ RPITIT 的 `+ Send` 把检查点前移 | `fail/rpitit_send_at_impl.rs` → `future cannot be sent between threads safely` |

## ★ 核心症状：那条 lint 把病因说出来了

```text
warning: use of `async fn` in public traits is discouraged as auto trait
         bounds cannot be specified
  = note: you can suppress this lint if you plan to use the trait only in your
          own code, or do not care about auto traits like `Send` on the Future
  = note: `#[warn(async_fn_in_trait)]` on by default
```

**"auto trait bounds cannot be specified"** —— `Send` / `Sync` 是 auto trait
（第 12 章），而 `async fn` 的简写形式**没有地方写它们**。

★ 注意 lint 说的是 **public traits**。只在 crate 内部用、不在乎 `Send` 时，
`#![allow(async_fn_in_trait)]` 是**合理**的。

## ★ 病因：返回类型是**不透明的**

```rust
pub trait Store { async fn get(&self, k: u64) -> u64; }
```

返回类型是**每个实现都不同的匿名状态机**（第 18 章）。
**三个症状，一个病因**：

| 需求 | 为什么不满足 |
|---|---|
| `dyn Store` | vtable 要固定布局，而每个实现的返回类型**大小不同** |
| `Send` bound | trait 只承诺"是 `Future`"，**没承诺是 `Send`** |
| 递归 | 状态机大小无穷（第 18 章 E0733） |

## ★ 好消息：泛型路径是**零分配 + 单态化**

MIR（`use_store_mem` 的 poll 实现）：

```mir
coroutine layout {
    field _s0: {async fn body of use_store<Mem>()};   // ★ 泛型被替换成具体类型
    ...
}
```

★ 注意 `use_store<Mem>` —— 单态化（第 7 章）。

汇编：

```asm
_use_store_mem:
	strb	wzr, [x8, #40]      ; 只写判别式
	ret
```

**没有 `Box`、没有 `alloc`。** AFIT 的泛型路径是零分配的。

> ⚠️ 旁证：**泛型函数不能 `#[unsafe(no_mangle)]`** ——
> rustc 警告 `functions generic over types or consts must be mangled`
> （`#[warn(no_mangle_generic_items)]`），因为它会被单态化成多份。

## ★ dyn 路线：`Box<dyn Future>`

```rust
pub trait StoreDyn {
    fn get(&self, k: u64) -> Box<dyn Future<Output = u64> + Send + '_>;
}
```

汇编（胖指针存进状态机）：

```asm
_use_store_dyn:
	stp	x0, x1, [x8]        ; ★ (data, vtable) 一起存
	strb	wzr, [x8, #40]
	ret
```

### ⚠️ 但它**不能直接 `.await`**

```text
error[E0277]: `dyn Future<Output = u64>` cannot be unpinned
  = note: required for `Box<dyn Future<Output = u64>>` to implement `Unpin`
```

**原因**：`.await` 的 blanket impl 要求 `F: Future + Unpin`，
而 **trait object 默认 `!Unpin`**（vtable 里没有 `Unpin` 的信息）。

**修法**：

```rust
let mut f = Box::into_pin(StoreDyn::get(s, 1));   // Box<dyn Future> → Pin<Box<dyn Future>>
std::future::poll_fn(|cx| f.as_mut().poll(cx)).await
```

★ 这是第 19 章的知识在**最意想不到的地方**冒出来：
"给异步 trait 加动态分发"这个看起来与 `Pin` 无关的需求，
**必然要过 `Pin` 这一关**。

## ★ 解法：RPITIT 手写 `+ Send`

```rust
pub trait StoreSend {
    fn get(&self, k: u64) -> impl Future<Output = u64> + Send;
}
```

代价：不能简写 `async fn`、每个实现都得手写 `impl Future` 包装。

**收益：`Send` 的检查点前移到实现处。**

| 写法 | 错误报在哪 | 好不好 |
|---|---|---|
| `async fn`（trait 没承诺 `Send`） | **调用点** | ❌ 离原因远 |
| RPITIT `+ Send`（签名承诺了） | **实现处** | ✅ 就在原因上 |

★ 对比第 20 章的"`Send` 沿 `.await` 传染、报错位置离原因很远" ——
**RPITIT 把这个问题提前到了原因所在的地方。**

## ★ 对照表：没有一列全是 ✅

| 写法 | 能 `dyn` | 能表达 `Send` | 分配 |
|---|---|---|---|
| `async fn` in trait | ❌ E0038 | ❌ | 无 |
| RPITIT `-> impl Future + Send` | ❌ | ✅ | 无 |
| `-> Box<dyn Future + Send>` | ✅ | ✅ | **一次堆分配** |

★ **三个需求最多同时满足两个。** 这就是 1.98 上 AFIT 的现状。

★ 而且 `dyn` 和 AFIT 的冲突是**根本性的**（不是"还没实现"）：
vtable 要**固定布局**，`async fn` 的返回类型每个实现都不同。
要支持得改 vtable 的模型本身 ——
**`Send` 已经有 RPITIT 解法了，`dyn` 还没有。**

## 反例四则

| 文件 | 错误 | 说明 |
|---|---|---|
| `fail/afit_not_dyn.rs` | **E0038** `not dyn compatible` | vtable 建不出来 |
| `fail/afit_not_send.rs` | `future cannot be sent between threads safely` | trait 没承诺 `Send` |
| `fail/box_dyn_future_not_awaitable.rs` | **E0277** `cannot be unpinned` | trait object 默认 `!Unpin` |
| `fail/rpitit_send_at_impl.rs` | 同上（但报在 **impl 处**） | ★ 检查点前移 |

## 交叉验证（可选）

```bash
grep -n 'field _s0: {async fn body of use_store<Mem>()}' .evidence/ch21-afit-lib.mir
awk '/^_use_store_mem:/{on=1} on{print} on&&/cfi_endproc/{exit}' .evidence/ch21-afit-lib.O3.s
awk '/^_use_store_dyn:/{on=1} on{print} on&&/cfi_endproc/{exit}' .evidence/ch21-afit-lib.O3.s
grep -n 'fn use_store' .evidence/ch21-afit-lib.mir | head
```

## 待办

- [x] 8 条断言全绿（`verify-all.sh ch21`）
- [x] 4 个反例落库并加断言
- [x] 实测确认泛型路径**零分配**（"异步 trait 有开销"是错的）
- [x] 实测确认 `Box<dyn Future>` 需要 `Box::into_pin`（第 19 章的交叉）
- [ ] ⚠️ **本章结论演进最快**：`return_type_notation`、
      `async fn in dyn trait` 等仍在推进，定稿前需复核
- [ ] 第 22 章（tokio）会展开 `Send + 'static` 在真实项目里的形态
