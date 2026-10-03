# 第 29 章：内存布局、有效值与 `MaybeUninit` - 实测证据

> 最后验证：**rustc 1.99.0** / LLVM 23.1.1 / `aarch64-apple-darwin`

## 当前工具链的布局观察

```bash
cargo test -p ch29-layout-validity
tools/evidence.sh ch29-layout-validity
```

| 类型 | size | align | 字段偏移 |
|---|---:|---:|---|
| `RustHeader` | 8 | 4 | `count=0, kind=4, tag=6` |
| `CHeader` | 12 | 4 | `tag=0, count=4, kind=8` |
| `PackedHeader` | 5 | 1 | `tag=0, count=1` |

第一行只是 rustc 1.99 的观察值；默认 `repr(Rust)` 没有稳定字段顺序。第二行由
目标平台的 C ABI 规则决定，才能用于与 C 共享布局。第三行消除了 padding，但
`count` 位于未对齐地址，不能创建 `&u32`。

## niche 不是“免费压缩技巧”，而是有效值推导

| 类型 | size |
|---|---:|
| `Option<u64>` | 16 |
| `Option<NonZeroU64>` | 8 |
| `Option<UserId>`（transparent） | 8 |
| `Option<bool>` | 1 |
| `Option<MaybeUninit<bool>>` | 2 |

`NonZeroU64` 的 0 和 `bool` 的 2..=255 都是无效值，编译器可以拿一个无效值编码
`None`。`MaybeUninit<bool>` 刻意允许任意未初始化/字节状态，外层 `Option` 因而
不能继续借用 bool 的 niche。

## 有效值反例

```text
packed_reference.rs   -> E0793: reference to packed field is unaligned
zeroed_reference.rs  -> invalid_value: references must be non-null
invalid_values.rs    -> Miri: constructing invalid value: encountered 0x02, but expected a boolean
```

这三条分别覆盖对齐、引用有效值和标量有效值。`MaybeUninit` 只允许“尚未把字节
解释成 T”；调用 `assume_init` 的瞬间，调用者必须证明初始化与有效值条件全部成立。

## `repr(C)` 仍然不是序列化格式

`CHeader` 含 padding，padding 字节可能未初始化；C ABI 还跟目标平台相关，并且
整数有端序。直接把整个结构体的内存当作网络字节流，会同时踩到未初始化读取、
跨平台布局和端序问题。序列化应逐字段编码，而不是 transmute 整块内存。
