# 5. 实战：写一个安全的树 / 图容器

> 一句话：**能用索引就用索引。**
> 树要能向上访问父节点，而 Rust 的所有权模型不允许"两个地方都拥有"。
> 有 `Rc<RefCell<>>`、arena + 索引、`unsafe` 三条路，
> 而**绝大多数情况正确答案是第二条**。

## 5.0 一个会让你卡住的例子

你想写一棵树，每个节点能访问自己的父节点：

```rust
struct Node {
    value: u64,
    parent: Option<???>,      // ← 这里填什么？
    children: Vec<Node>,
}
```

`parent` 填 `&Node`？那谁拥有它？生命周期怎么标？
填 `Box<Node>`？那父子互相拥有，直接无限递归。

**这是第 1–4 章所有概念的交汇点**：
- 所有权（第 1 章）：一个值只能有一个所有者；
- 生命周期（第 2 章）：引用不能比被引用者活得久；
- variance（第 3 章）：`&mut` 的不变性会传染；
- 移动（第 4 章）：自引用结构一移就悬垂。

**树要向上访问，天然就是"自引用 + 共享"。** 这是 Rust 里最经典的难题之一。

## 5.1 三条路线

```rust
// 路线 1：共享所有权 + 运行时借用检查
pub struct RcNode {
    value: u64,
    parent: Option<Rc<RefCell<RcNode>>>,
    children: Vec<Rc<RefCell<RcNode>>>,
}

// 路线 2：arena + 索引（推荐）
pub struct NodeId(pub usize);          // 只是个整数
pub struct ArenaNode {
    value: u64,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

// 路线 3：unsafe + 裸指针（本章不展开，见第 24 章）
pub struct RawNode {
    value: u64,
    parent: Option<*mut RawNode>,
    children: Vec<*mut RawNode>,
}
```

先看路线 2 为什么好。

## 5.2 编译器眼里的样子

### 5.2.1 同一个操作，指令数差 4 倍

两种实现做同一件事：从某个节点向上走到根，返回深度。

**arena 版本**（`arena_depth`，`-O`，共 **24** 条指令）：

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
	madd	x8, x8, x10, x9     ; ← 索引 * 48 + base：一条乘加搞定寻址
	ldr	w11, [x8]
	ldr	x8, [x8, #8]        ; 读 parent
	add	x0, x0, #1
	tbnz	w11, #0, LBB13_1    ; Option 的 niche 判断
	ret
```

**`madd`（multiply-add）是整段的核心**：arena 寻址就是一次乘法。
没有引用计数、没有借用标志、没有 panic 路径、没有析构。

**`Rc<RefCell<>>` 版本**（`rc_depth`，`-O`，共 **104** 条指令）。
从它的汇编里能直接读到三个符号：

| 符号 | 说明 |
|---|---|
| `alloc::rc::Rc<RefCell<RcNode>>::drop_slow` | 引用计数归零时走的**慢路径** |
| `core::cell::panic_already_mutably_borrowed` | **`RefCell` 的运行时借用检查**失败路径 |
| `core::panicking::panic_in_cleanup` / `__Unwind_Resume` | 上面两条带来的**异常安全代码** |

**`RefCell::borrow()` 不是零成本的。** 它在运行时检查"当前有没有可变借用"，
失败就 panic。`panic_already_mutably_borrowed` 是**慢路径**，
但检查本身在快路径上——一个计数器比较，加上分支。

### 5.2.2 结构体大小

```rust
size_of::<RcNode>()    = 40    // value:8 + parent:8 + children:24
size_of::<ArenaNode>() = 48    // value:8 + parent:16 + children:24
```

看起来 arena 更大？因为 **`Option<usize>` 是 16 字节**（没有 niche，
需要额外的 tag 字节）。用 `u32` + 哨兵就能压回去：

```rust
pub struct ArenaNode32 { value: u64, parent: u32, children: Vec<u32> }
size_of::<ArenaNode32>() = 40    // u32::MAX 表示"没有父节点"
```

**这是 arena 的另一个好处**：索引可以任意小。
`Rc` 永远做不到——指针就是 8 字节。

而且 `RcNode` 的 40 字节只是**栈上/内联**的部分。
每个 `Rc` 还指向一块堆内存，里面额外存着 **两个引用计数**（strong + weak）。
**每个节点一次堆分配 + 16 字节计数。**

### 5.2.3 `Rc` 的循环引用问题

```rust
let parent = RcNode::new(1);
let child = RcNode::add_child(&parent, 2);
```

`child` 里有一个 `Rc` 指向 `parent`，`parent` 里有一个 `Rc` 指向 `child`。
两个引用计数都至少是 1。**离开作用域时谁也不会归零** → 内存泄漏。

修法是 `Weak`：

```rust
pub parent: Option<Weak<RefCell<RcNode>>>,   // 不增加引用计数
```

但 `Weak` 又要 `upgrade()`（返回 `Option`，失败要处理），
**代码复杂度又上一层**。

**arena 天然没有这个问题**：`NodeId` 只是整数，
节点全部归 `Arena` 的 `Vec` 拥有。drop 掉 `Arena`，整棵树一起走。

## 5.3 为什么 arena 通常是最优解

**因为它把"所有权问题"变成了"索引问题"。**

- `NodeId` 是 `Copy` 的整数 → **第 1–4 章的所有借用规则都不参与**；
- 节点的所有者只有一个：`Arena` 的 `Vec`；
- 向上/向下访问都是"查表"，不需要所有权转移；
- 内存连续（`Vec<ArenaNode>` 是一块），缓存友好；
- 可以用小整数索引压缩节点（见 5.2.2）。

**唯一的真正限制**：`NodeId` 一旦离开 `Arena` 就无意义。
你不能把节点单独返回给调用者，也不能放进另一个容器。

### 什么时候 `Rc<RefCell<>>` 才是对的

- 节点需要**独立于容器存活**（比如返回给调用者、跨模块传递）；
- 结构是**动态共享**的（同一个节点被多处引用，且无法用索引表达）；
- 节点数量小、性能不敏感。

**实践判据**：如果你能回答"这个节点属于谁"，就用 arena；
如果答不上来，才考虑 `Rc`。

## 5.4 反直觉的点

### 反直觉之一：更"Rust 风格"的写法反而更慢

`Rc<RefCell<>>` 是官方文档里常见的"共享可变"方案，看起来更"正统"。
但它的指令数是 arena 的 **4 倍**，而且带 panic 路径和析构逻辑。

**Rust 的"零成本抽象"不适用于 `Rc<RefCell<>>`。**
它不是零成本的——它是**用运行时开销换类型系统的表达力**。

这和第 2、4 章的结论形成对照：
- 生命周期、`Pin`：**纯类型层，零运行时成本**；
- `Rc<RefCell<>>`：**真有运行时成本**（引用计数 + 借用标志）。

**区分这两类，是"深水区"的一个实用技能。**

### 反直觉之二：索引比引用"更强"，因为它不受生命周期约束

```rust
// 引用版本：编译不过
fn get_two<'a>(t: &'a Tree) -> (&'a Node, &'a Node) {
    (&t.left_child, &t.left_child)      // 借用检查器可能不满意
}

// 索引版本：随便返回
fn get_two_ids(t: &Tree) -> (NodeId, NodeId) {
    (t.left, t.left)                    // NodeId 是 Copy，没有借用
}
```

**索引没有生命周期**，所以能表达引用表达不了的关系（比如"返回一个指向
自己内部的句柄，稍后再解析"）。这是**类型层的自由度**，不只是性能。

### 反直觉之三：arena 的"不安全"是错觉

有人担心"用索引访问 `Vec` 会 panic"。
确实——`self.nodes[id.0]` 会做边界检查（见 `arena_depth` 汇编里的 `cmp`/`b.hs`）。

但这不是 arena 特有的问题：`Rc` 版本的 `borrow()` 也会 panic
（`panic_already_mutably_borrowed`）。**两条路线都有失败路径**，
区别是 arena 的失败是"索引越界"（可以用 `get()` 返回 `Option`），
`Rc` 的失败是"借用冲突"（更难预测，取决于运行时的借用历史）。

## 5.5 亲手验证

```bash
tools/evidence.sh ch05-project-tree
scripts/verify-all.sh ch05

# 对照指令数
awk '/^_arena_depth:/,/cfi_endproc/' .evidence/ch05-project-tree-lib.O3.s | grep -vc '^\s*\.\|^_'
awk '/^_rc_depth:/,/cfi_endproc/'    .evidence/ch05-project-tree-lib.O3.s | grep -vc '^\s*\.\|^_'

# 看 Rc 版本的三笔开销
grep -o 'drop_slow\|panic_already_mutably_borrowed\|__Unwind_Resume' \
     .evidence/ch05-project-tree-lib.O3.s | sort -u
```

**怎么算验证成功**：

1. `arena_depth` 的汇编里有 `madd` —— 索引寻址是一条乘加；
2. `rc_depth` 的汇编里出现 `drop_slow`、`panic_already_mutably_borrowed`、
   `__Unwind_Resume` —— 三笔运行时开销；
3. 两个函数的指令数差 4 倍左右。

```bash
scripts/verify-all.sh ch05      # 5 条断言
```

## 5.6 与 unsafe 的关系

路线 3（裸指针）能做到和 arena 一样的性能，甚至更快（省掉边界检查）。
但代价是**你要自己保证**：

- 节点在被访问时还活着（不能悬垂）；
- 没有两个 `&mut` 同时指向一个节点（第 24 章的 `noalias`）；
- 删除节点时没有别的指针还指着它。

**这正好是第 1–4 章讲的四件事**：
所有权、生命周期、别名、移动。
`unsafe` 不是"关掉检查"，是"把这些检查挪到你脑子里"。

**建议**：先写 arena 版本，profile 之后如果边界检查真的是瓶颈
（几乎不会），再考虑 `unsafe`。**不要一开始就上 `unsafe`。**

## 5.7 小结

- **树/图的本质困难**：向上访问 = 自引用 + 共享所有权，
  与第 1–4 章的规则直接冲突。
- **三条路线**：`Rc<RefCell<>>`（共享 + 运行时检查）、
  arena + 索引（纯整数）、`unsafe`（自己保证）。
- **默认选 arena**：`NodeId` 是 `Copy`，所有借用规则都不参与；
  零运行时开销；内存连续；无循环引用问题。
- **`Rc<RefCell<>>` 的三笔开销**在汇编里可见：
  `drop_slow`（引用计数）、`panic_already_mutably_borrowed`（借用检查）、
  `__Unwind_Resume`（异常安全）。指令数差 4 倍。
- **`Option<usize>` 是 16 字节**，用 `u32` + 哨兵可以压缩节点大小——
  这是引用版本做不到的。
- **区分两类抽象**：生命周期/`Pin` 是**零运行时成本**的；
  `Rc<RefCell<>>` **不是**。这是"深水区"最实用的一个判断。
- **索引比引用"更强"**：它不受生命周期约束，能表达引用表达不了的关系。

第 1 部分到此结束。四章的基础（借用检查、生命周期、variance、移动）
加上一个实战，构成后面所有内容的地基。

下一部分进入 trait 系统——从"关联类型 vs 泛型参数"开始。
那个选择比大多数人想的更本质：**它决定了"一个类型能实现几次这个 trait"**。
