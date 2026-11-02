# 29. 内存布局、有效值与 `MaybeUninit`

> 一句话：拥有足够多的字节不代表已经拥有一个 `T`；地址还要满足对齐，字节要
> 已初始化，bit pattern 也必须是 `T` 的有效值。

## 先把四个条件分开

从裸内存构造一个 `T`，至少要同时满足：

1. **大小**：有 `size_of::<T>()` 个可访问字节；
2. **对齐**：地址满足 `align_of::<T>()`；
3. **初始化**：读取到的每一位都已经初始化；
4. **有效值**：bit pattern 被 `T` 允许。

`u8` 的所有 256 个 bit pattern 都有效；`bool` 只有 0 和 1；引用必须非空、对齐，
并指向满足其访问契约的对象；枚举判别值也只能落在合法范围内。

这些规则是优化器契约。即使程序“暂时没用到”一个非法值，只要把无效 bit pattern
构造成对应类型，通常已经进入 UB。

### 放到业务里：从网络包直接转成结构体

下面这种代码同时假设了布局、对齐、初始化、有效值和端序：

```rust,ignore
let header: &Header = unsafe { &*(bytes.as_ptr().cast()) };
```

`bytes` 长度足够只是第一关。若 Header 含 `bool`、enum、引用或 padding，外部字节
可能根本不能表示一个有效 Header。因此协议解析通常应逐字段读取并验证。

## 29.0 `repr(Rust)`：能观察，不能写进 ABI

默认结构体使用 `repr(Rust)`。语言保证字段适当对齐、不重叠，类型整体满足对齐；
它不承诺字段保持源码顺序，也不承诺不同编译器版本得到同一布局。

本章的样本：

```rust
pub struct RustHeader {
    pub tag: u8,
    pub count: u32,
    pub kind: u16,
}
```

rustc 1.99 当前观察到：

```text
size=8, align=4, count@0, kind@4, tag@6
```

编译器重排后只需 8 字节。这是解释性能与内存占用的证据，不是可以对外承诺的
ABI。升级工具链后，自动断言变化意味着“观察结果变了”，不等于编译器破坏兼容。

## 29.1 `repr(C)`：稳定字段规则，不是无 padding

同样字段加上 `#[repr(C)]`：

```rust
#[repr(C)]
pub struct CHeader {
    pub tag: u8,
    pub count: u32,
    pub kind: u16,
}
```

当前目标结果为：

```text
size=12, align=4, tag@0, count@4, kind@8
```

`tag` 后有 3 字节 padding，末尾还有 2 字节 tail padding。`repr(C)` 的承诺是按
目标 C ABI 排列，不是“紧凑”“跨所有平台相同”，更不是“可以直接序列化”。

padding 还可能包含未初始化字节。把整个结构体转成 `&[u8]` 并读取 padding，可能
触发 UB；即使恰好全初始化，端序和目标 ABI 仍让它不适合作为网络格式。

## 29.2 niche：无效值也是一种信息

`NonZeroU64` 排除 0，所以 `Option<NonZeroU64>` 可以用 0 表示 `None`，无需额外
判别字节：

| 类型 | 本章实测大小 |
|---|---:|
| `Option<u64>` | 16 |
| `Option<NonZeroU64>` | 8 |
| `Option<UserId>`，其中 UserId 为 transparent wrapper | 8 |
| `Option<bool>` | 1 |
| `Option<MaybeUninit<bool>>` | 2 |

这种供枚举编码使用的无效值空间称为 **niche**。最后一行尤其关键：
`MaybeUninit<bool>` 允许内部处于任意字节状态，所以 bool 原有的无效值不能再被
外层 `Option` 占用，大小反而变成 2。

因此有效值规则不仅限制 unsafe，也直接参与正常 Rust 的数据布局与优化。

## 29.3 `repr(transparent)` 的精确用途

单字段 newtype 可以用 `#[repr(transparent)]` 承诺与内部非零大小字段具有相同
布局和 ABI：

```rust
#[repr(transparent)]
pub struct UserId(std::num::NonZeroU64);
```

它适合在 FFI 或内部 ABI 边界上给原始类型加静态语义。它不允许任意多字段；额外
字段必须是零大小且不能改变对齐。transparent 也不自动证明所有转换 sound，内部
类型的有效值约束仍然存在：整数 0 不能构造 `UserId`。

## 29.4 `MaybeUninit<T>` 真正允许了什么

普通局部变量一旦具有类型 `T`，编译器就可以假设它是有效 `T`。因此不能先造一个
“假的 T”，稍后再填：

```rust,ignore
let reference: &u64 = unsafe { std::mem::zeroed() }; // 当场 UB
```

`MaybeUninit<T>` 的作用是延迟这份承诺。它与 `T` 大小、对齐相同，但其内部存储
暂时不受 `T` 的初始化和有效值约束：

```rust
let mut slot = std::mem::MaybeUninit::<u64>::uninit();
slot.write(42);
let value = unsafe { slot.assume_init() };
assert_eq!(value, 42);
```

`write` 之后 `assume_init` 合法。若漏掉 write，或写入字节 2 后把它解释成 bool，
`assume_init` 就是在向编译器撒谎。

## 29.5 初始化与析构必须一起记账

对 `MaybeUninit<String>` 调用 `write` 后，那块内存里已经有一个需要析构的 String，
但 `MaybeUninit` 自己不会自动 drop 内部值。这是必要设计：它不知道内部究竟是否
完成初始化。

因此批量初始化数组或容器时，需要显式维护“前多少个元素已初始化”：

```text
写入 slot[0] -> initialized = 1
写入 slot[1] -> initialized = 2
中途 panic      -> 只 drop [0..initialized)
全部成功        -> 把所有权转换成 [T; N]，guard 不再清理
```

这就是上一章 drop flag 的手写版本。第 30 章会在 `ArrayVec<T, N>` 中完整实现这套
不变式，并用 Miri 验证正常路径、满容量错误和析构路径。

## 29.6 `repr(packed)` 与未对齐引用

`#[repr(C, packed)]` 可以消除 padding，但字段地址可能不满足自身类型的对齐。
引用永远必须对齐，因此下面代码即使只读取也被拒绝：

```rust,compile_fail,E0793
#[repr(C, packed)]
struct Header { tag: u8, count: u32 }

fn count_ref(header: &Header) -> &u32 {
    &header.count
}
```

需要先用 `addr_of!` 形成裸指针，再显式 `read_unaligned`。不要为了节省几个 padding
字节就把内部热路径结构全部 packed；未对齐访问可能更慢，API 也更危险。

## 29.7 编译器与 Miri 各能看见什么

本章有三组反例：

- `&packed.field`：rustc 报 E0793，因为不对齐可静态确定；
- `mem::zeroed::<&u64>()`：`invalid_value` lint 能静态指出空引用无效；
- 动态写入字节 2，再 `assume_init::<bool>()`：静态分析看不穿，Miri 在执行时抓到。

这再次说明检查工具是互补的。Miri 只检查实际走到的路径；没有报错不能推广为对
所有输入都 sound。

### 29.7.1 从布局到系统边界：让 C 真正调用 Rust

`repr(C)` 约束数据布局，不自动指定函数 ABI。一个 C 接口还需规定符号名、
调用约定、有效值和错误表达。`src/ffi.rs` 提供完整最小接口：

```rust,ignore
#[repr(C)]
pub struct AddResult {
    pub value: i32,
    pub status: u8,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_deep_checked_add(left: i32, right: i32) -> AddResult {
    match left.checked_add(right) {
        Some(value) => AddResult { value, status: 0 },
        None => AddResult { value: 0, status: 1 },
    }
}
```

加法溢出返回状态 1，不把 panic 作为跨语言错误协议。接口使用固定宽度整数，
不返回 String、Vec、引用或 trait object，也不假装一次 null 检查就能证明任意
外部指针可安全访问。

```bash
bash examples/ch29-layout-validity/c-ffi/run.sh
```

脚本把 `src/ffi.rs` 编译为 native staticlib，再用系统 C 编译器构建并运行调用方。
C 与 Rust 各自验证偏移和大小，并实际调用正常 / 溢出路径；不比较 padding。
探针进入跨平台门禁，在 Linux / macOS 原生运行，不只是生成交叉目标文件。

指针接口还必须规定长度、对齐、生命周期、可变性与释放责任；允许 unwind 的
ABI 也必须双方明确约定。`extern "C"` 本身不会验证这些条件。

## 29.8 亲手验证

```bash
cargo test -p ch29-layout-validity
tools/evidence.sh ch29-layout-validity
scripts/verify-evidence.sh ch29

# 可选：非法 bool 的动态有效值检查
scripts/verify-miri.sh
```

应观察到 C/Rust 布局差异、五种 Option 大小、两个编译失败反例，以及 Miri 对
非法 bool 的报错。

## 29.9 小结

- `repr(Rust)` 的当前布局可以测量，但不能作为稳定 ABI；
- `repr(C)` 固定 C ABI 规则，仍可能有 padding、端序和平台差异；
- 类型的无效值可以成为 enum 的 niche，这就是许多 Option 不增大的原因；
- `MaybeUninit<T>` 延迟“这里已经是有效 T”的承诺，不会替调用者完成证明；
- `repr(packed)` 字段可能未对齐，不能直接创建引用；
- unsafe 初始化代码必须同时维护初始化进度与析构进度。
