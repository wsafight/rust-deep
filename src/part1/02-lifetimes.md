# 2. 生命周期：标注、省略与推断

> 一句话：`'a` **不是时间**。它是约束求解里的一个变量，
> 编译器解一组不等式，然后把结果**全部丢掉**——它在最终代码里不存在。

## 2.0 一个会让你卡住的例子

```rust
fn longest(x: &str, y: &str) -> &str {
    if x.len() > y.len() { x } else { y }
}
```

```text
error[E0106]: missing lifetime specifier
 = help: this function's return type contains a borrowed value,
         but the signature does not say whether it is borrowed from `x` or `y`
```

"缺生命周期标注"。于是你加上 `'a`：

```rust
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() > y.len() { x } else { y }
}
```

编译过了。但你现在大概有一个说不清的地方：

**`'a` 到底是什么？** 是"x 和 y 活得一样久"吗？可 `x` 和 `y` 明明可以来自
完全不同的地方。是"取两者中较短的那个"吗？那为什么写出来是同一个 `'a`？

这一章要回答的就是这个。

## 2.1 表层解释（官方书会怎么讲）

官方书会说：

- `'a` 是"生命周期参数"，表示引用有效的范围；
- 有省略规则，所以很多时候不用写；
- 标注不会改变引用的实际寿命，只是"告诉编译器你的意图"。

最后一句是对的，但它太弱了。真正该说的是：

> **标注不改变任何东西——它不改变运行时的任何东西，
> 它甚至不改变编译器的"判断"，它只是把约束写下来。**

## 2.2 编译器眼里的样子

### 2.2.1 先看：生命周期**不产生任何代码**

这是本章最硬的一条证据。看这两个函数：

```rust
#[unsafe(no_mangle)] pub fn with_lifetime<'a>(x: &'a str) -> usize { x.len() }
#[unsafe(no_mangle)] pub fn without_lifetime(x: &str) -> usize { x.len() }
```

生成 LLVM IR：

```bash
rustc --edition 2024 -O --emit llvm-ir=out.ll --crate-type=lib \
      examples/ch02-lifetimes/src/lib.rs
```

```llvm
@without_lifetime = unnamed_addr alias i64 (ptr, i64), ptr @with_lifetime
```

**LLVM 判定这两个函数逐位等价，把后者折叠成了前者的 alias。**
在汇编里更直白——`_without_lifetime` 连函数体都没有：

```asm
_with_lifetime:
	mov	x0, x1        ; 只取 str 的长度字段，字符串指针根本没碰
	ret

_without_lifetime = _with_lifetime
```

同一件事在"显式标注 vs 省略"上更明显：

```llvm
@longest_elided = unnamed_addr alias { ptr, i64 } (ptr, i64, ptr, i64), ptr @longest
```

`longest` 和 `longest_elided` 的**唯一区别**就是签名里写没写 `'a`——
LLVM 认为它们是同一个函数。

> **所以"生命周期有没有运行时开销"是个范畴错误。**
> 不是"开销很小"，是**根本不参与运行时**。
> 借用检查器用完就把它们丢了。

### 2.2.2 那它们在哪一步起作用？

只在**借用检查**那一步。MIR 生成之后、codegen 之前。
借用检查器读的是 MIR 上的数据流事实（第 1 章），
而生命周期是它用来**描述**这些事实的语言。

具体来说，编译器做的事情是：

1. 从你的签名和函数体里收集一组**约束**；
2. 解这些约束（如果无解 → 报错）；
3. 解完就丢，只留下"通过 / 不通过"这个结论。

### 2.2.3 约束长什么样：错误信息直接告诉你

**例一：缺标注。**

```text
 = help: the signature does not say whether it is borrowed from `x` or `y`
```

★ 措辞是"**借用自谁**"，不是"活多久"。
编译器要的是**关系**——返回值跟着哪个输入走。

**例二：推断器临时起的名字。**

```rust
impl<'a> Parser<'a> {
    pub fn get2(&self, other: &str) -> &str { other }
}
```

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

★ **`'1` / `'2` 是你根本没写过的名字。**
它们是推断器在求解过程中临时起的。这就是"生命周期是**推断**出来的"最直接的证据：
你写的是**约束**，编译器给出的是**解**。

**例三：错误信息本身就是一条不等式。**

```rust
pub fn to_static<'a>(x: &'a str) -> &'static str { x }
```

```text
error: lifetime may not live long enough
  | pub fn to_static<'a>(x: &'a str) -> &'static str { x }
  |                -- lifetime `'a` defined here      ^ returning this value
  |                                                    requires that `'a` must
  |                                                    outlive `'static`
```

★ **"requires that `'a` must outlive `'static`" 读作 `'a: 'static`。**
约束求解的输出就是一组这样的不等式。

### 2.2.4 所以 `longest<'a>(x: &'a str, y: &'a str) -> &'a str` 是什么意思

它**不是**说"x 和 y 活得一样久"，也不是"取两者较短的那个"。

它是说：**存在某个生命周期 `'a`，使得 `x: &'a str`、`y: &'a str`、返回值也是 `&'a str`。**

关键在于：**`'a` 是调用点上被推断出来的**。
调用 `longest(a, b)` 时，`'a` 会被推断成"覆盖 `a` 和 `b` 的那段"，
也就是**两者中较短的那个**。

所以你的直觉"取较短的那个"**在结果上是对的**，
但机制不是"编译器比较了两个长度"，而是"求解器找到了一个同时满足所有约束的值"。

> 这个区别不是抠字眼。它解释了为什么下面这段能编译：
> ```rust
> let s: &'static str = "literal";
> let local = String::from("temp");
> let r = longest(s, &local);      // 'a 被推断成 local 的生命周期
> ```
> `'static` 的 `s` 被塞进了需要"更短"的位置。如果 `'a` 是"必须一样长"，
> 这就编译不过了。**约束是单向的不等式，不是等号。**

## 2.3 为什么必须这样设计

为什么不能"给每个引用算一个具体的时间区间"？

**因为运行时根本没有那个信息。** 引用的有效期是**控制流**的属性，
不是值的属性：一个引用在某个分支里可能被释放，在另一个分支里不会。
要"算时间"，就得先知道程序会走哪条分支——那就是不可判定问题了。

所以编译器换了策略：

> 不解"活多久"，只解"谁必须比谁长"。

这是个**偏序**上的约束满足问题，是可判定的。
代价是它必须保守：只要存在一条路径违反约束，就拒绝（第 1 章）。

## 2.4 省略规则：不是魔法，是默认选择

三条规则：

| 规则 | 例子 | 展开成 |
|---|---|---|
| 每个输入引用各得一个生命周期 | —— | —— |
| **只有一个输入**生命周期 | `fn f(x: &str) -> &str` | `fn f<'a>(x: &'a str) -> &'a str` |
| **有 `&self`** | `fn get(&self) -> &str` | `fn get<'s>(&'s self) -> &'s str` |
| 其他 | —— | 必须显式标注 |

第三条是 `Parser::get` 能编译的原因：

```rust
impl<'a> Parser<'a> {
    pub fn get(&self) -> &str { self.s }        // 输出跟 &self 走
}
```

**而 `get2` 就失败**，因为规则 2 强行规定"输出跟 `&self`"，
可函数体返回的却是 `other`：

```rust
pub fn get2(&self, other: &str) -> &str { other }   // 报错
```

**这证明省略规则只是一个默认选择**，一旦默认选择与函数体冲突，照样报错。
它不是"编译器帮你补全了正确的标注"，而是"编译器替你猜了一个，猜错了就报错"。

## 2.5 反直觉的点

### 反直觉之一：生命周期不是"时间"，所以"活得长"不是唯一方向

```rust
pub fn outlives<'a, 'b: 'a>(x: &'a str, _y: &'b str) -> &'a str { x }
```

`'b: 'a` 读作"`'b` 比 `'a` 长"。这里的 `_y` 活得**更久**，却完全不参与输出。

对比一个常见的错误直觉："返回值不能活得比参数长"。准确说法是：
**返回值的生命周期必须被某个输入的约束覆盖**——不是"不能长"，是"必须有人担保"。

### 反直觉之二：标注可以**缩短**一个引用的"使用寿命"

```rust
let s: &'static str = "literal";
let local = String::from("temp");
let r = longest(s, &local);      // s 是 'static，但 'a 被推断成 local 的生命周期
```

`s` 明明能活到程序结束，但在这个调用点上它**被当作**短命的用。
这不是"改变"了 `s` 的生命周期，而是"`'a` 求解成较短的那个值"。
**约束是单向的：长的可以当短的用（协变），反过来不行。**

这个"长的能不能当短的用"的问题，就是第 3 章的 variance。

### 反直觉之三：标注写"紧"了会让**更多**代码编译不过

这一条有个干净的例子（都实测过）：

```rust
// 写法 A：把两个输入绑到同一个 'a
fn tied<'a>(x: &'a str, _y: &'a str) -> &'a str { x }

// 写法 B：只让输出跟 x 绑，y 随便
fn precise<'a>(x: &'a str, _y: &str) -> &'a str { x }
```

两个函数体完全一样。但用法不同：

```rust
pub fn use_precise() -> &'static str {
    let local = String::from("temp");
    precise("static str", &local)     // ✅ 编译过：'a 求解成 'static
}

pub fn use_tied() -> &'static str {
    let local = String::from("temp");
    tied("static str", &local)        // ❌ E0515
}
```

```text
error[E0515]: cannot return value referencing local variable `local`
  |     tied("static str", &local)
  |     ^^^^^^^^^^^^^^^^^^^------^ returns a value referencing data owned by
  |                                the current function
```

**为什么**：写法 A 的 `'a` 同时约束 `x` 和 `y`，
而 `&local` 只能满足一个很短的生命周期 —— 求解器只好把 `'a` 拉短到
`local` 的寿命。于是返回值 `&'a str` 也就变成了短命的，不能返回。
写法 B 里 `y` 不受 `'a` 约束，`'a` 就能保持 `'static`。

**`'a` 不是"让编译器更宽容"，是你主动写下的一个承诺。**
承诺越紧（约束越多），求解空间越小，可能被拒的情况越多。

**实践含义**：函数签名里少写一点生命周期，往往比多写更好。
编译器默认给的（省略规则）通常就是**最宽松**的那个。

## 2.6 亲手验证

```bash
# 生成证据
tools/evidence.sh ch02-lifetimes

# 看"生命周期被擦除"的铁证
grep '^@without_lifetime\|^@longest_elided' .evidence/ch02-lifetimes-lib.O3.ll

# 看汇编里根本没有函数体
grep -A3 '^_without_lifetime' .evidence/ch02-lifetimes-lib.O3.s

# 三个反例
for f in missing_lifetime elision_ambiguous outlives; do
  rustc --edition 2024 --crate-type=lib \
        examples/ch02-lifetimes/fail/$f.rs 2>&1 | head -3
done
```

**怎么算验证成功**：

1. LLVM IR 里出现 `@without_lifetime = ... alias ... ptr @with_lifetime`
   —— 带不带 `'a` 是**同一个函数**；
2. 汇编里 `_without_lifetime = _with_lifetime` 是个 alias，**没有函数体**；
3. 三个反例分别报 E0106 / `lifetime may not live long enough` ×2。

```bash
scripts/verify-all.sh ch02      # 7 条断言
```

## 2.7 与 unsafe 的关系

生命周期被擦除，**不代表保证被擦除**。

借用检查器丢掉的是"名字"，留下的是**义务**：

- `&'a T` 在 `'a` 期间有效 → 这条义务变成 LLVM 的 `dereferenceable`、
  `nonnull`、`noalias` 元数据（第 24 章）；
- 一旦你用 `unsafe` 造出一个"生命周期比实际更长"的引用
  （`transmute`、裸指针、`mem::forget` 配合），
  编译器的这些元数据就成了**假的** —— 那是 UB。

Miri 能抓到它：

```text
error: Undefined Behavior: memory access failed: alloc291 has been freed,
       so this pointer is dangling
```

**`'a` 是编译器和你之间的合同**。`unsafe` 里你要自己保证合同不被违反。

## 2.8 小结

- **生命周期不是时间，是约束**。编译器解一组不等式（`'a: 'b`），不是量长度。
- **约束求解的输出是一组不等式**——错误信息里那些
  "requires that `'a` must outlive `'static`" 就是它。
- **推断器会临时起名**（`'1` / `'2`），这证明生命周期是**推断**出来的，
  你的标注只是**输入**。
- **生命周期在 codegen 里完全不存在**：带 `'a` 和不带 `'a` 的函数
  被 LLVM 折叠成 alias。所以"生命周期有开销"是范畴错误。
- **`longest<'a>(x: &'a str, y: &'a str) -> &'a str` 的含义是"存在 `'a` 同时满足三条约束"**，
  调用点上 `'a` 被推断成两者中较短的那个。结果和"取较短"一致，但机制不同——
  机制是求解，不是比较。
- **省略规则是默认选择，不是补全**。默认选择与函数体冲突时报错（`get2`）。
- **标注是承诺，不是宽容**：写得多，可能被拒得多。

下一章我们看这个"长的能不能当短的用"的规则本身：
协变、逆变与不变。
