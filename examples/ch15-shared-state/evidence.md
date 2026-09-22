# 第 15 章：共享可变状态的所有权设计 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch15-shared-state
scripts/verify-all.sh ch15       # 5 条断言（PASS=7）
```

## 关键结论与断言（5 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | `atomic` 版本存在 | `.O3.s` 的 `^_atomic_accumulate:` |
| 2 | `Mutex` 版本存在 | `.O3.s` 的 `^_mutex_accumulate:` |
| 3 | ★ `Mutex` 带 poison 检查（atomic 没有） | `.O3.s` 里 `GLOBAL_PANIC_COUNT` |
| 4 | `Rc<RefCell>` 在单线程里可用 | `.O3.s` 的 `^_local_refcell:` |
| 5 | `Rc<RefCell>` 不能跨线程 | `fail/rc_refcell_thread.rs` → **E0277** |

## ★★ 核心证据：同样一次累加，代价差近三倍

### `Arc<AtomicU64>` 的循环体（**完整**）

```asm
LBB38_2:
	add	x9, x0, #16
	ldadd	x8, x9, [x9]        ; ← 全部的工作
	subs	x1, x1, #1
	b.ne	LBB38_2
```

**4 条指令。没有函数调用、没有 poison 检查、没有异常安全代码。**

### `Arc<Mutex<u64>>` 的循环体

```asm
LBB42_1:
	subs	x20, x20, #1        ; 循环计数
	b.eq	LBB42_16
	ldapr	x0, [x21]           ; ★ pthread Mutex 的 OnceBox 初始化状态
	cbz	x0, LBB42_9
LBB42_3:
	bl	__RNv...pal4unix4sync5mutexNtB2_5Mutex4lock    ; ★ 函数调用
	ldr	x8, [x22]           ; ★ 读 GLOBAL_PANIC_COUNT（poison 检查）
	tst	x8, #0x7fffffffffffffff
	b.ne	LBB42_10            ; 有 panic 过 → 走 poison 路径
	ldrb	w8, [x19, #24]      ; 检查 poison 标志
	cbnz	w8, LBB42_26
	ldr	x8, [x19, #32]      ; 读数据
	add	x8, x8, #1          ; ← 真正的工作：就这一条
	str	x8, [x19, #32]      ; 写数据
LBB42_7:
	...
	bl	__RNv...Mutex6unlock  ; ★ 又要一次函数调用
```

**11 条指令 + 两次函数调用，只为做一次 `+1`。**

| | 每次自增的循环体 |
|---|---|
| `Arc<Mutex<u64>>` | **11 条指令 + 2 次 `bl`** |
| `Arc<AtomicU64>` | **4 条指令** |

★ 而且这还不包括 `Mutex4lock` / `Mutex6unlock` **函数体内部**的指令
（pthread 调用、错误检查、fast path 判断），也不包括异常安全代码。

### `Mutex` 的记账清单（这就是"贵"在哪）

从上面的循环体能直接读出 `Mutex` 提供的**全套服务**：

1. `ldapr` + `cbz` —— pthread Mutex 包装层的懒初始化检查；
2. `bl ...Mutex4lock` —— 函数调用（可能有竞争 → 进 pthread 阻塞）；
3. `GLOBAL_PANIC_COUNT` 的读 + `tst` + 分支 —— **poison 检查**；
4. poison 标志位检查（`ldrb` + `cbnz`）；
5. 异常安全代码（`MutexGuard::drop` 要解锁）；
6. `bl ...Mutex6unlock` —— 又一次函数调用。

实测 `GLOBAL_PANIC_COUNT` 在这个文件里出现 **4 次**。

> **"我只是想加个 1" —— 但 `Mutex` 不知道你想干什么，
> 它必须为"任何事"做好准备。**

## 各设计的函数体指令数（粗粒度对照）

```bash
for f in owned_sum shared_read_sum mutex_accumulate atomic_accumulate chunked_sum local_refcell; do
  n=$(awk "/^_$f:/,/cfi_endproc/" .evidence/ch15-shared-state-lib.O3.s | grep -cE '^\s+[a-z]')
  echo "$f: $n"
done
```

| 函数 | 指令数 | 说明 |
|---|---|---|
| `atomic_accumulate` | **24** | 含整个 `Arc` 的 drop 路径 |
| `local_refcell` | **12** | 单线程，最便宜 |
| `mutex_accumulate` | **141** | 含 poison / 异常安全 / 两次函数调用 |
| `owned_sum` | 309 | 含线程创建 + join |
| `shared_read_sum` | 328 | 含 `Arc` clone + 线程创建 |
| `chunked_sum` | 513 | 含 `available_parallelism` + 多线程 |

⚠️ **这张表不能直接当"性能排名"读**：
这些函数做的工作量不同（有的只算一次累加，有的创建线程并 join）。
**有意义的是上面那个"循环体"对照**（同样的工作，4 条 vs 11 条）。

> 本书没有接入对应 benchmark，因此这里只比较结构和生成代码。
> 这里只说"哪个做了更多事"。

## 反例：`fail/rc_refcell_thread.rs`

```text
error[E0277]: `Rc<RefCell<u64>>` cannot be sent between threads safely
   --> examples/ch15-shared-state/fail/rc_refcell_thread.rs:32:24
    |
 32 |       std::thread::spawn(move || {
    |       ------------------ ^------
 34 | |     })
    | |_____^ `Rc<RefCell<u64>>` cannot be sent between threads safely
    |
    = help: within `{closure@...}`, the trait `Send` is not implemented for `Rc<RefCell<u64>>`
```

**这不是"`Rc<RefCell>` 不好"，而是"它的设计目标就是单线程"。**

| | `Rc<RefCell<T>>` | `Arc<Mutex<T>>` |
|---|---|---|
| 引用计数 | 普通 `add`（1 条） | 原子 `ldadd` |
| 借用检查 | **运行时**计数（违反时 panic） | 阻塞等待 + poison |
| 能跨线程 | ❌ | ✅ |

★ `src/lib.rs` 的 `local_refcell` **编译通过**，函数体只有 **12 条指令**
—— 单线程里它比 `Arc<Mutex<T>>` 便宜得多。
**选对工具的前提是选对"共享范围"。**

## 判据表（本章结论）

| 层次 | 类型 | 谁能改 | 代价 | 什么时候用 |
|---|---|---|---|---|
| **0** | `T`（move） | 唯一的持有者 | **零** | 数据是"一次性"的 |
| **1** | `Arc<T>` | **没人**改 | 一次 `ldadd` | 读多写无 |
| **2** | `Arc<AtomicU64>` | 共享，但只有"一个值"可改 | 4 条指令 | 计数器 / 标志位 |
| **3** | `Arc<Mutex<T>>` | 谁都能改**任意**状态 | 11+ 条指令 + 函数调用 | 真的需要共享可变 |
| **4** | `Arc<RwLock<T>>` | 读并发、写独占 | 更贵 | 读远多于写**且**状态复杂 |

**决策顺序**：

1. **能不能不共享？**（move / channel / 每线程一份）→ 层次 0 或 4；
2. **共享之后要改吗？** 不改 → `Arc<T>`；
3. **要改，但只是一个整数 / 标志？** → `AtomicXxx`；
4. **要改任意状态？** → `Arc<Mutex<T>>`，并考虑分片。

## 交叉验证（可选）

```bash
# 两种累加器的循环体对照
awk '/^_atomic_accumulate:/,/cfi_endproc/' .evidence/ch15-shared-state-lib.O3.s
awk '/^_mutex_accumulate:/,/cfi_endproc/'  .evidence/ch15-shared-state-lib.O3.s

# Mutex 的 poison 记账
grep -c 'GLOBAL_PANIC_COUNT' .evidence/ch15-shared-state-lib.O3.s
```
