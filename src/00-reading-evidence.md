# 0. 先读懂证据：MIR、LLVM IR 与汇编

> 目标：看到证据时，知道该看哪三行，也知道它不能证明什么。
> 前置：能读简单 Rust 函数。建议阅读时间：15–20 分钟。

## 0.1 先确定问题，再定位函数

想验证“最后使用引用之后可以修改原值”，就选第 1 章的 `simple()`，
不要从整个项目 MIR 的第一页开始读：

```bash
tools/mir.sh ch01-borrow
```

在 `.evidence/ch01-borrow-lib.mir` 中查找 `fn simple`，先确认函数签名，
再找借用产生、读取与修改这三条语句：

```text
_2 = &_1;
_3 = copy ((*_2).0: f64);
(_1.0: f64) = const 3f64;
```

## 0.2 MIR 速查：局部槽位与基本块

| 记号 | 如何读 |
|---|---|
| `_0` | 当前函数的返回值位置 |
| `_1`、`_2` | 参数或局部槽位；编号不是变量生命周期 |
| `debug name => _1` | 源码名称与槽位的调试映射 |
| `scope` | 调试作用域，不表示借用一定活到这里结束 |
| `bb0`、`bb1` | 基本块；块内顺序执行，末尾转向其他块或返回 |
| `copy place` | 复制允许复制的值；复制引用不等于复制它指向的数据 |
| `move place` | 转移值，不是调用移动构造器，也不保证实际搬运字节 |
| `StorageLive` / `StorageDead` | 某些 MIR 阶段的存储范围标记，不是完整借用分析结果 |
| `drop(place)` | 对仍被拥有的值执行析构 |

MIR 能解释这个样本的数据流，但不能把“最后一次使用”简单推广到带返回引用、
析构器的全部程序。`--emit=mir` 也不是 borrow checker 内部事实的完整 dump。

## 0.3 先画箭头，再读控制流

```text
bb0: call(...) -> [return: bb1, unwind: bb2];
bb1: return;
bb2 (cleanup): drop(...) -> ...;
```

调用的 `return` 边指向正常返回后继续执行的块，`unwind` 边指向 panic 展开的
清理路径。第 27 章的 drop flags 决定哪些值仍需析构；`resume` 表示继续向外
传播 panic。`panic=abort` 下不能期待同样的展开路径。

其他常见终结语句包括 `goto`、`switchInt` 和 `return`。读循环先找回边；读异步
状态机先看判别值跳到哪个状态，不必一次理解全部临时槽位。

## 0.4 LLVM IR：限定函数，再找属性

```bash
tools/llvm.sh ch24-noalias 3
```

从 `define` / `declare` 的函数签名读起。`ptr` 表示指针，`i64` 是 64 位整数，
`load` / `store` 是读取 / 写入，`call` 是调用，`br` 是分支，`ret` 是返回。
`noalias`、`readonly` 等属性是编译器交给优化器的契约，不是运行时检查。
不能在整个文件 grep 到一个词，就断言目标函数具有相应性质。

## 0.5 汇编：先读输入、访存、调用与返回

```bash
tools/asm.sh ch07-vtable 3
```

| 用途 | AArch64 常见写法 | x86_64 常见写法 |
|---|---|---|
| 传送值 | `mov` | `mov` / `movq` |
| 读 / 写内存 | `ldr` / `str` | 带内存操作数的 `mov` |
| 成对读 / 写 | `ldp` / `stp` | 依指令序列决定 |
| 加法 / 比较 | `add` / `cmp` | `add` / `cmp` |
| 直接调用 / 返回 | `bl` / `ret` | `call` / `ret` |
| 间接跳转 | `br xN` / `blr xN` | `jmp *...` / `call *...` |
| acquire / release 访存 | `ldar`、`ldapr` / `stlr` | 普通 `mov` 也可能承担这些语义 |
| 原子读改写 | `ldadd`、`cas` 的相应变体 | `lock xadd`、`lock cmpxchg` |

AArch64 的 `[x0, #24]` 是以 x0 为基址、偏移 24 字节的地址。AT&T 风格的
x86_64 `%rdi` 是寄存器，`24(%rdi)` 是访存地址。不同语法的操作数顺序不同，
参数寄存器也依赖 ABI，不能直接照搬。

一条 `stp` 能写两个寄存器，不表示它是一次原子事务，也不表示它比两条 `str`
快一倍。指令数量不是延迟、吞吐量或真实业务性能的替代品。

## 0.6 复现纪律与练习

记录 rustc、target 与 opt-level；O0 调试结构和 O3 优化结果不能混着比较。
汇编只支持当前样本的代码生成结论；Miri 与 Loom 也有不同边界，见
[附录 D](./appendix/d-verification.md)。

不想先配置完整工具链，可以在 [Rust Playground](https://play.rust-lang.org/)
尝试小型独立样本，或在 [Compiler Explorer](https://godbolt.org/) 查看代码生成。
先确认可选版本和目标平台；网站默认配置不能替代本书固定目标的断言。

- 复制 `&String` 会复制堆上的文本吗？不会，复制的是引用。
- 把读取 first 的 `println!` 从 push 前移到后，为什么可能报 E0502？因为修改
  可能使旧引用失效。到第 1 章验证。
- x86_64 少一条指令就一定更快吗？不一定，硬件、指令和负载都不同。到第 16 章
  区分指令映射与协议正确性。
