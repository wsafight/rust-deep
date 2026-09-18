# 第 6 章：关联类型 vs 泛型参数 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch06-associated-types
scripts/verify-all.sh ch06      # 7 条断言
```

## ★ 核心判据：一个类型能实现这个 trait 几次

### 关联类型 → 一次

```rust
pub trait Container { type Item; fn get(&self, i: usize) -> Option<&Self::Item>; }

impl Container for Numbers { type Item = u64; ... }
impl Container for Numbers { type Item = String; ... }   // ← 第二次
```

```text
error[E0119]: conflicting implementations of trait `Container` for type `Numbers`
  |
3 | impl Container for Numbers { type Item = u64; ... }
  | -------------------- first implementation here
4 | impl Container for Numbers { type Item = String; ... }
  | ^^^^^^^^^^^^^^^^^^^^ conflicting implementation for `Numbers`
```

★ **`Self::Item` 是 `Self` 的函数**：一个 `Self` 只能有一个 `Self::Item`。
这是"关联类型"的**定义**，不是限制。

### 泛型参数 → 任意多次

```rust
pub trait Conv<T> { fn conv(self) -> T; }

impl Conv<u64>    for Wrapper { fn conv(self) -> u64    { self.0 as u64 } }
impl Conv<f64>    for Wrapper { fn conv(self) -> f64    { self.0 as f64 } }
impl Conv<String> for Wrapper { fn conv(self) -> String { self.0.to_string() } }
```

三个 impl **全部编译通过**。

## ★ 代价：泛型参数带来**类型推断歧义**

```rust
let w = Wrapper(5);
let _y = w.conv();      // 要 u64 还是 f64？
```

```text
error[E0283]: type annotations needed
  |
4 |     let _y = w.conv();
  |                 ^^     ---- type must be known at this point
  |
note: multiple `impl`s satisfying `Wrapper: Conv<_>` found
  |
2 | impl Conv<u64> for Wrapper { fn conv(self) -> u64 { self.0 as u64 } }
3 | impl Conv<f64> for Wrapper { fn conv(self) -> f64 { self.0 as f64 } }
help: consider giving `_y` an explicit type
```

**对照**：同一个 `Wrapper`，只要上下文能确定类型，就不用标注：

```rust
pub fn conv_with_annotation(w: Wrapper) -> u64 { w.conv() }   // ✅ 返回类型确定了 impl
```

**关联类型完全没有这个问题** —— `Self::Item` 是唯一的，不需要推断：

```rust
pub fn use_container(c: &Numbers) -> u64 { *c.get(0).unwrap_or(&0) }   // ✅ 无需标注
```

## 生成的代码：两者都是零成本

`conv_with_annotation`（泛型参数，单态化）：
```asm
_conv_with_annotation:
	mov	w0, w0
	ret
```

`use_container`（关联类型）：
```asm
_use_container:
	ldp	x9, x8, [x0, #8]
	...                              ; 边界检查 + 取值
```

**两者都完全内联/单态化，没有间接调用。**
（对比第 7 章的 `dyn`：那里是 `ldr x1, [x1, #24]` + `br x1`。）

**结论**：关联类型 vs 泛型参数的差别**不在运行时**，
而在**类型层**——表达能力（能实现几次）和推断难度。

## 判据总结（正文的结论）

| | 关联类型 `type Item` | 泛型参数 `trait Tr<T>` |
|---|---|---|
| 一个类型能实现几次 | **1** | 任意多次 |
| 调用点类型标注 | **不需要** | 常常需要 |
| 语义 | "这个类型的**那个**关联类型" | "这个类型对**某个** T 的实现" |
| 典型例子 | `Iterator::Item`、`Deref::Target` | `From<T>`、`PartialEq<Rhs>` |

**选择判据（一句话）**：
> 问自己"**一个类型有几种合理答案？**"
> 只有一种 → 关联类型；可能多种 → 泛型参数。

- `Iterator::Item`：一个 `Vec<i32>` 迭代出什么？只有 `i32`。→ 关联类型
- `From<T>`：`i32` 能从什么转来？`u8`、`u16`、`f64`…… 多种。→ 泛型参数
- `Deref::Target`：`Box<i32>` 解引用成什么？只有 `i32`。→ 关联类型

## 待办

- [x] 断言 7 条全绿
- [ ] 第 7 章：`dyn` 与 `object safety`（`examples/ch07-vtable` 已有基础）
- [ ] 第 10 章（GAT）会回到这一章：关联类型**带参数**之后，
      表达力向泛型参数靠拢，但仍保持"一个类型一次实现"
