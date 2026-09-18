# 第 2 章：生命周期 —— 标注、省略与推断 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch02-lifetimes
scripts/verify-all.sh ch02      # 8 条断言
```

## ★ 核心证据：生命周期在 codegen 里**完全不存在**

```rust
#[unsafe(no_mangle)] pub fn with_lifetime<'a>(x: &'a str) -> usize { x.len() }
#[unsafe(no_mangle)] pub fn without_lifetime(x: &str) -> usize { x.len() }
```

LLVM IR（`.evidence/ch02-lifetimes-lib.O3.ll`）：
```llvm
@without_lifetime = unnamed_addr alias i64 (ptr, i64), ptr @with_lifetime
```

**LLVM 判定这两个函数逐位等价，把后者折叠成了前者的 alias。**
再看汇编，`_without_lifetime` 甚至**没有函数体**：
```asm
_with_lifetime:
	mov	x0, x1        ; 只取长度字段，字符串指针根本没碰
	ret

_without_lifetime = _with_lifetime
```

**讲法**：
- 生命周期是**借用检查器用的**，检查完就丢；
- 它不进入 MIR 之后的任何阶段，**不产生任何代码**；
- 所以"生命周期有运行时开销"这个担心是**范畴错误** ——
  它不是"开销小"，是**根本不参与运行时**。

## ★ 第二组：显式标注 vs 省略，生成同一个函数

```rust
pub fn longest<'a>(x: &'a str, y: &'a str) -> &'a str { ... }
pub fn longest_elided<'a>(x: &'a str, y: &'a str) -> &'a str { ... }
```

```llvm
@longest_elided = unnamed_addr alias { ptr, i64 } (ptr, i64, ptr, i64), ptr @longest
```

汇编里 `longest` 只有 4 条指令：
```asm
_longest:
	cmp	x1, x3
	csel	x1, x1, x3, hi
	csel	x0, x0, x2, hi
	ret
```
（两个 `csel` 是"取更长的那个"——`str` 是胖指针，指针和长度各选一次。）

**`'a` 只影响"能不能编译"，不影响"编译出什么"。**

## ★ 第四组：过度标注反而编译不过

```rust
fn tied<'a>(x: &'a str, _y: &'a str) -> &'a str { x }      // 两个输入绑同一个 'a
fn precise<'a>(x: &'a str, _y: &str) -> &'a str { x }      // 只有输出跟 x 绑
```

函数体完全一样，但：

| 用法 | `tied` | `precise` |
|---|---|---|
| `fn demo() -> &'static str { let local = ...; f("static str", &local) }` | ❌ E0515 | ✅ 编译过 |

**原因**：`tied` 的 `'a` 同时约束 `x` 和 `y`，而 `&local` 只能满足很短的寿命
→ `'a` 被拉短 → 返回值不能是 `'static`。
`precise` 里 `y` 不参与 `'a`，`'a` 保持 `'static`。

**教训**：标注写得越紧，求解空间越小。省略规则给的通常是最宽松的。

## 四个反例：错误信息的措辞就是本章的论点

### 1. `fail/missing_lifetime.rs` —— E0106

```text
error[E0106]: missing lifetime specifier
 = help: this function's return type contains a borrowed value,
         but the signature does not say whether it is borrowed from `x` or `y`
```

★ 注意措辞：**"没有说它借用自 x 还是 y"**。
编译器不是在问"活多久"，而是在问"**跟着谁**"。
生命周期是**关系**，不是时长。

### 2. `fail/elision_ambiguous.rs` —— 推断器给的临时名字

```text
error: lifetime may not live long enough
  |     pub fn get2(&self, other: &str) -> &str { other }
  |                 -             -               ^^^^^ method was supposed to
  |                 |             |                     return data with lifetime `'2`
  |                 |             |                     but it is returning data
  |                 |             |                     with lifetime `'1`
  |                 |             let's call the lifetime of this reference `'1`
  |                 let's call the lifetime of this reference `'2`
```

★ **`'1` / `'2` 是你根本没写过的名字** —— 它们是推断器临时起的名。
这是"生命周期是求解出来的"最直接的证据：编译器在**推断**，
你的标注只是**约束**。

### 3. `fail/outlives.rs` —— 错误信息就是一条不等式

```text
error: lifetime may not live long enough
  | pub fn to_static<'a>(x: &'a str) -> &'static str { x }
  |                -- lifetime `'a` defined here      ^ returning this value
  |                                                    requires that `'a` must
  |                                                    outlive `'static`
```

★ **"requires that `'a` must outlive `'static`" 读作 `'a: 'static`。**
约束求解的输出就是一组这样的不等式。

### 4. `fail/over_annotated.rs` —— 过度标注（见上一节）

```text
error[E0515]: cannot return value referencing local variable `local`
  |     tied("static str", &local)
  |     ^^^^^^^^^^^^^^^^^^^------^ returns a value referencing data owned by
  |                                the current function
```

## 省略规则（三条，都能验证）

| 规则 | 例子 | 展开成 |
|---|---|---|
| 只有一个输入生命周期 | `fn f(x: &str) -> &str` | `fn f<'a>(x: &'a str) -> &'a str` |
| 有 `&self` | `fn get(&self) -> &str` | `fn get<'s>(&'s self) -> &'s str` |
| 其他 | —— | 必须显式标注（否则 E0106） |

**省略规则不是"魔法"**，它只是"在歧义时给个默认选择"。
`elision_ambiguous.rs` 证明了：一旦默认选择与函数体冲突，照样报错。

## ⚠️ 本章证据链的局限（写作时必须知道）

**variance（协变/逆变/不变）无法从 codegen 层面取证**，原因：

- `-Zdump-variance` 在 1.98 **不存在**（实测 `rustc -Z help` 里没有）；
- `#[rustc_variance]` 属性已不对外可用（报 `cannot find attribute`）；
- `RUSTC_LOG` 的 variance target 在 release 构建里被静态禁用到 `info` 级别，
  拿不到 debug 输出；
- 而 variance 本身**不产生任何代码**（它只在类型检查期起作用）。

**唯一可用的证据是编译期行为**：某些赋值能过、某些不能过。
建议把它们放到第 3 章（`ch03-variance`），用"能编译 / 不能编译"的对照来演示，
**不要在正文里承诺"能看到 variance 的输出"**。

## 待办

- [x] 断言 8 条全绿
- [ ] 第 3 章：建 `ch03-variance` example，用编译期对照演示协变/不变/逆变
- [ ] 第 4 章（自引用与 Pin 的前置知识）需要新 example
