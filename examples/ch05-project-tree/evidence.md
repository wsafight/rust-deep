# 第 5 章实战：树 / 图容器 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch05-project-tree
scripts/verify-all.sh ch05
```

## ★ 核心对照：`Rc<RefCell<>>` vs arena + 索引

同一个操作（向上遍历到根，返回深度），两种实现的**指令数差 4 倍**：

| | 指令数（`-O`） | 调用了什么 |
|---|---|---|
| `rc_depth` | **104** | `Rc::drop_slow`、`panic_already_mutably_borrowed`、`panic_in_cleanup`、`__Unwind_Resume` |
| `arena_depth` | **24** | 无 |

`arena_depth` 的全部核心代码：

```asm
_arena_depth:
	mov	x8, x1              ; id
	mov	x9, x0              ; arena
	mov	x0, #0              ; depth = 0
	ldp	x9, x1, [x9, #8]    ; nodes.ptr, nodes.len
	mov	w10, #48            ; sizeof(ArenaNode)
LBB13_1:
	cmp	x8, x1
	b.hs	LBB13_4             ; 边界检查
	madd	x8, x8, x10, x9     ; ← 索引 * 48 + base：一条乘法搞定寻址
	ldr	w11, [x8]
	ldr	x8, [x8, #8]        ; 读 parent
	add	x0, x0, #1
	tbnz	w11, #0, LBB13_1    ; Option 的 niche 判断
	ret
```

**`madd`（乘加）是整段的核心**：arena 寻址就是一次乘法。
没有引用计数、没有借用标志、没有 panic 路径、没有析构。

## ★ `Rc<RefCell<>>` 的三笔额外开销

从 `rc_depth` 的汇编里能直接读到三个符号：

| 符号 | 说明 |
|---|---|
| `alloc::rc::Rc<RefCell<RcNode>>::drop_slow` | **引用计数归零时**才走的慢路径（需要判断） |
| `core::cell::panic_already_mutably_borrowed` | **`RefCell` 的运行时借用检查失败**路径 |
| `core::panicking::panic_in_cleanup` / `__Unwind_Resume` | 上面两条带来的**异常安全代码** |

**注意第二行**：`RefCell::borrow()` 会在运行时检查"当前有没有可变借用"。
这个检查**不是零成本的**，而且失败时会 panic。
（`panic_already_mutably_borrowed` 是**慢路径**，
但检查本身在快路径上——一个计数器比较。）

## ★ 结构体大小

```rust
pub struct RcNode {
    value: u64,
    parent: Option<Rc<RefCell<RcNode>>>,      // Rc = 一个指针（8 字节）
    children: Vec<Rc<RefCell<RcNode>>>,       // Vec = 3 个指针（24 字节）
}
// size_of::<RcNode>() = 40
```

`Rc<T>` 就是**一个指针**——但每个节点一次堆分配，
而且 `RcNode` 的 40 字节里，每个 `Rc` 指向的堆块还要额外存
**两个引用计数**（strong + weak，各 8 字节）。

arena 版本：

```rust
pub struct ArenaNode {
    value: u64,
    parent: Option<usize>,       // ← 16 字节！
    children: Vec<usize>,
}
// size_of::<ArenaNode>() = 48
```

**`Option<usize>` 是 16 字节**（没有 niche，需要额外的 tag）。
用 `u32` + 哨兵（`u32::MAX` = 无父）可以压到 **40 字节**：

```rust
pub struct ArenaNode32 { value: u64, parent: u32, children: Vec<u32> }
// size_of = 40
```

→ **arena 的另一个好处**：可以用小整数索引，把节点压小。
`Rc` 版本永远做不到——指针就是 8 字节。

## 三条路线的取舍（正文的结论）

| 路线 | 向上访问 | 代价 | 什么时候用 |
|---|---|---|---|
| `Rc<RefCell<>>` | ✅ | 引用计数 + 运行时借用检查 + 每节点一次堆分配 | 树的结构**动态且共享**、节点数量小 |
| **arena + 索引** | ✅ | 无（纯整数） | **绝大多数情况**——推荐默认 |
| `unsafe` + 裸指针 | ✅ | 自己保证不变式 | 需要零开销且能给出完整论证 |

**推荐默认是 arena**，理由：
1. 索引是 `Copy`，没有所有权问题——**第 1–4 章的所有借用规则都不参与**；
2. 零运行时开销（`madd` 一条指令）；
3. 内存连续，缓存友好（`Vec<ArenaNode>` 是一块连续内存）；
4. `Rc` 的循环引用需要 `Weak` 才能避免泄漏——arena 天然没有这个问题。

**`Rc<RefCell<>>` 唯一真正的优势**：节点可以在**脱离 arena** 的情况下独立存活
（比如返回给调用者、放进另一个容器）。arena 的 `NodeId` 一旦离开 arena 就无意义。

## 待办

- [x] 断言 5 条全绿
- [ ] 补一个图的例子（带环），演示 `Rc` 的循环引用泄漏
- [ ] 第 2 部分（trait 系统）从第 6 章开始
