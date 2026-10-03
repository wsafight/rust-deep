# 30. 实战：实现一个安全的 `ArrayVec<T, N>`

> 一句话：unsafe 容器的核心不是指针技巧，而是把“哪些位置已经初始化、谁负责
> 析构它们”写成一个始终成立的不变式。

## 需求与边界

我们实现一个固定容量、元素内联存储的向量：

```rust
pub struct ArrayVec<T, const N: usize> {
    len: usize,
    data: [std::mem::MaybeUninit<T>; N],
}
```

目标包括：

- `push`、`pop`、`insert`、`remove`；
- 借成 `&[T]` / `&mut [T]`；
- 按引用和按值迭代；
- 初始化中途失败、迭代中途放弃时仍正确析构；
- `N=0`、零大小类型和有析构器的类型都能工作；
- 整个容器不进行堆分配。

它不是标准库 `Vec` 的替代品。它的价值是把前面三章的 drop flag、有效值与
`MaybeUninit` 放进同一个可审计的抽象。

## 30.0 先写不变式，再写 unsafe

`ArrayVec<T, N>` 的完整安全不变式只有三条：

```text
1. len <= N
2. data[..len] 恰好是已初始化、有效、由容器拥有的 T
3. data[len..] 不得作为 T 读取或析构
```

这三条必须在每个 public 方法返回时成立，也必须在任何可能 unwind 的位置成立。
代码审查时不用“整体感觉没问题”，只需逐个 unsafe block 检查它依赖哪一条、
执行后是否仍保持三条。

## 30.1 为什么不能写 `[T; N]`

创建空容器时没有 N 个 T 可以填入。用 `[T; N]` 就只能要求 `T: Default`，而且会
无意义地构造再覆盖所有元素。

`[MaybeUninit<T>; N]` 表示“这些槽位具有 T 的大小和对齐，但尚未承诺里面是 T”：

```rust,ignore
pub const fn new() -> Self {
    Self {
        len: 0,
        data: [const { MaybeUninit::uninit() }; N],
    }
}
```

此处没有 unsafe，因为我们还没有读取任何槽位，也没有声称它们已经初始化。

## 30.2 push：先写值，再发布长度

```rust,ignore
pub fn try_push(&mut self, value: T) -> Result<(), T> {
    if self.len == N { return Err(value); }
    self.data[self.len].write(value);
    self.len += 1;
    Ok(())
}
```

顺序不能随意交换。如果先递增 len，再在构造元素时发生 panic，`Drop` 会把那个
未初始化槽位当作 T 析构。当前 `MaybeUninit::write` 本身不会 panic，但保持
“写完再增加 len”的结构能让以后扩展代码时仍然容易审计。

满容量时返回 `Err(value)`，把所有权明确还给调用者；静默 drop 或泄漏 value 都
会让 API 的资源语义变得模糊。

## 30.3 pop：先缩短长度，再移出值

`pop` 的顺序与 push 相反：

```rust,ignore
self.len -= 1;
let value = unsafe { self.data[self.len].assume_init_read() };
```

新的 len 先把槽位移出容器的析构责任，再由 `assume_init_read` 转移 T 的所有权。
如果未来在返回路径加入可能 panic 的逻辑，`ArrayVec::drop` 也不会重复析构它。

`assume_init_read` 不能对同一槽位调用两次：每次调用都声称“这里有一个由我接管
的独立 T”。第二次读取会复制所有权，最终 double free。

## 30.4 insert/remove：重叠移动后的尾部是什么

插入时，已初始化后缀要整体向右移动一格。源区与目标区重叠，所以必须使用
`ptr::copy`，而不是要求不重叠的 `copy_nonoverlapping`：

```rust,ignore
ptr::copy(base.add(index), base.add(index + 1), len - index);
base.add(index).write(value);
len += 1;
```

删除则先 `read` 目标元素，再把右侧元素向左覆盖，最后缩短 len。移动之后，旧的
最后一个槽位仍残留相同比特，但它已经位于 `data[len..]`：不再是一个可读取或
析构的 T。这里“残留字节”和“仍拥有一个值”不是同一件事。

## 30.5 从已初始化前缀构造 slice

`MaybeUninit<T>` 的数组不能直接借成 `[T]`。依据不变式，可以从底层指针构造只
覆盖前缀的 slice：

```rust,ignore
unsafe { slice::from_raw_parts(data.as_ptr().cast::<T>(), len) }
```

证明义务逐项对应第 29 章：

- 数组指针非空且满足 T 的对齐；
- len 不超过 N；
- 前 len 个槽位已经初始化；
- 这些 bit pattern 都来自安全的 T 值；
- `&self` / `&mut self` 分别保证共享或独占访问期限。

实现 `Deref<Target=[T]>` 后，排序、二分查找和切片迭代都能直接复用标准库。

## 30.6 Drop：只清理前缀

整个 `[MaybeUninit<T>; N]` 被析构时不会自动析构内部 T。因此 `ArrayVec` 的
`Drop` 必须显式构造 `data[..len]` 的 raw slice，并调用 `drop_in_place`。

绝对不能遍历全部 N 个槽位；后缀没有初始化，尝试 drop 就是 UB。也不能忘掉前缀，
否则带资源的 T 会泄漏。

`clear` 复用 `pop`：每次先缩短 len，再 drop 返回值。如果某个 `T::drop` panic，
尚未处理的前缀仍由更新后的 len 描述，容器不变式没有被破坏。

## 30.7 失败中的部分初始化

`try_from_fn` 逐个调用可能失败的构造函数。这里不需要额外 guard：正在构建的
`ArrayVec` 自身就是 guard。

```rust,ignore
let mut out = ArrayVec::new();
while out.len() < N {
    let value = make(out.len())?;
    out.try_push(value).unwrap();
}
```

`?` 提前返回或 `make` panic 时，`out` 的 Drop 只清理已经写入的前缀。这正是
编译器 drop flag 在自定义容器里的显式版本。

## 30.8 按值迭代要转移析构责任

`IntoIterator for ArrayVec` 不能一边让原容器 drop，一边把元素交给 iterator。
实现用 `ManuallyDrop<ArrayVec>` 阻止原析构器，再把整个 `MaybeUninit` 数组移入
`IntoIter`：

```text
ArrayVec owns [0..len)
        | ManuallyDrop + ptr::read(data)
        v
IntoIter owns [next..end)
```

每次 `next()` 先增加 next，再 `assume_init_read` 当前元素；iterator 提前被丢弃时，
它自己的 Drop 清理 `[next..end)`。已经 yield 的前缀归调用者，未初始化后缀没人
读取，所有区域的所有权恰好分配一次。

## 30.9 零容量和 ZST 不是边角问题

泛型 unsafe 容器必须考虑：

- `N=0`：任何 push 都应返回原值，slice 仍须合法；
- `T=()` 等零大小类型：逻辑上有多个元素，即使它们地址可能相同；
- `T` 带 Drop：每个逻辑元素仍必须恰好析构一次。

本章测试覆盖这三类。不要用“指针地址不同”来表示 ZST 元素是否存在，应始终以
`len` 和索引范围作为所有权账本。

## 30.10 亲手验证

```bash
cargo test -p ch30-arrayvec
tools/evidence.sh ch30-arrayvec
scripts/verify-evidence.sh ch30

# 对所有 ArrayVec 测试再跑一遍解释执行
scripts/verify-miri.sh
```

汇编样本 `sum_arrayvec([u64; 4])` 不包含堆分配。MIR 中可以看到 write、
`assume_init_read`、重叠 `ptr::copy` 和前缀 `drop_in_place`。但当前 rustc 没有把
包装完全优化掉：裸数组版本使用 SIMD，`ArrayVec` 版本仍有栈帧和标量加法。
**零堆分配是可验证事实，“零抽象开销”则不能仅凭设计意图宣称。**

## 30.11 小结

- unsafe 容器首先是一个不变式，不是若干裸指针操作；
- len 同时记录可读取范围和析构责任，更新顺序决定 panic safety；
- `MaybeUninit` 后缀可以有任意字节，但绝不能当作 T 读取或析构；
- 移动后残留的字节不再代表一个被拥有的值；
- 按值 iterator 必须从容器接管未消费元素的析构责任；
- Miri 是路径级检查，不替代对所有泛型参数和控制流的不变式论证。
