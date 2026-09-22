# 7. dyn vs 泛型：dyn compatibility 与单态化代价

> 一句话：在无法去虚化的简单调用点，`dyn` 的直接调用成本通常表现为
> **从 vtable 读取函数指针，再做一次间接跳转**。更大的潜在代价是优化器
> 可能无法继续内联。
> 而"什么 trait 能做 `dyn`"这个问题，答案也在 vtable 的**布局**里。

## 先把语法认清

`fn run<T: Service>(s: &T)` 使用静态分发：编译器为具体 `T` 单态化。
`fn run(s: &dyn Service)` 使用动态分发：值由数据指针和 vtable 指针组成，
运行时通过表中的函数指针调用。`Box<dyn Service>` 则把具体值放到堆上，
常用于需要统一存储多种实现的场景。

泛型像为每种类型单独裁一套衣服，`dyn` 像统一走总机转接。前者容易内联，
后者能把不同实现放进同一个容器；没有谁天然高级，只有边界放得对不对。

### 放到业务里：插件集合与高频调用内核

插件注册表通常需要 `Vec<Box<dyn Plugin>>`，因为插件类型在运行前不统一；
序列化热循环或数值内核则更适合泛型，让 LLVM 有机会内联和向量化。真实设计
往往两者并用：系统边界用 `dyn` 保持可扩展，内部热路径转成具体类型或枚举。

```rust
let plugins: Vec<Box<dyn Plugin>> = load_plugins();
for plugin in &plugins { plugin.on_request(&request); }
```

这里插件类型直到配置加载后才知道，`dyn` 换来的运行时可替换性通常比一次
间接调用更重要。

## 7.0 一个会让你卡住的例子

你写了一个 trait，想拿它做 trait object：

```rust
pub trait Cache {
    fn get(&self, key: &str) -> Option<String>;
    fn insert<T: ToString>(&mut self, key: &str, value: T);   // ← 泛型方法
}

let cache: Box<dyn Cache> = ...;
```

```text
error[E0038]: the trait `Cache` is not dyn compatible
  |
5 | let cache: Box<dyn Cache> = ...;
  |            ^^^^^^^^^^^^ `Cache` is not dyn compatible
  |
note: for a trait to be dyn compatible it needs to allow building a vtable
note: ...because method `insert` has generic type parameters
```

"不能建 vtable"。但 `insert` 看起来只是个普通方法——它为什么妨碍建表？

换个写法也报错：

```rust
pub trait Cloneable {
    fn clone_me(&self) -> Self;      // ← 返回 Self
}
```

```text
note: ...because method `clone_me` references the `Self` type in its return type
```

**两种看起来无关的写法，报的是同一类错误。** 这一章要说明它们为什么是同一件事。

## 7.1 先把常见说法摆上桌

通常会这样概括：

- object safety（现在叫 **dyn compatibility**）要求：方法不能有泛型参数、
  不能返回 `Self`、不能有 `where Self: Sized` 之外的 `Self` 用法；
- `dyn Trait` 用一个 vtable 做动态分发；
- 泛型是单态化，`dyn` 是动态分发，后者有开销。

这些都对。但"为什么泛型方法不行"这一条，书里往往只给结论。
**理由其实很具体**：vtable 是**一张固定大小的表**。

## 7.2 编译器眼里的样子

### 7.2.1 vtable 长什么样（本章实测）

```asm
	.section	__DATA,__const
l_anon.9cb3917e37f35be02d8e38bfb2db0eeb.0:
	.asciz	"\000\000\000\000\000\000\000\000\b\000\000\000\000\000\000\000\b\000\000\000\000\000\000"
	.quad	_call_generic                                          ; ← 偏移 24：area
	.quad	__RNvXCsrtIYgyWToU_3libNtB2_2SqNtB2_5Shape4name        ; ← 偏移 32：name
```

逐字节读：

| 偏移 | 内容 |
|---|---|
| 0 | `drop_in_place`（这里全是 `\0`，因为 `Sq` 没有析构） |
| 8 | `size = 8`（`\b`） |
| 16 | `align = 8`（`\b`） |
| **24** | 第一个方法 `area` |
| **32** | 第二个方法 `name` |

**vtable 就是"3 个固定槽 + N 个方法槽"的定长数组。**

调用侧：

```asm
_dyn_area:
	ldr	x1, [x1, #24]     ; ← 从固定偏移取函数指针
	br	x1                ; ← 间接跳转
```

**"固定偏移"是关键词。** 因为偏移固定，才能用一条 `ldr` 取到。

### 7.2.2 泛型方法为什么破坏这个布局

`fn insert<T: ToString>(&mut self, key: &str, value: T)` ——
每个 `T` 都需要**一个不同的函数**（单态化）：

- `insert::<String>`、`insert::<u32>`、`insert::<MyType>`……

这些函数体不同、地址不同。**它们要占几个槽位？**

- 一个槽位？那 `dyn Cache` 调用 `insert` 时，怎么知道该调哪个 `T`？
- 每个 `T` 一个槽位？那 `T` 有无穷多个，**表的大小无法确定**。

**vtable 是编译期生成的定长结构，而泛型方法的实例数量在编译期不确定。**
两者直接冲突。

所以编译器说："**needs to allow building a vtable**"——
它的意思就是"我没法为这个 trait 生成一张定长的表"。

### 7.2.3 返回 `Self` 为什么也不行

`fn clone_me(&self) -> Self` —— 在 `dyn Cloneable` 的上下文里，
`Self` 就是 `dyn Cloneable` 本身，而**`dyn Cloneable` 的大小未知**
（`Self: ?Sized`）。

vtable 里有 `size` 和 `align` 两个槽位，正是为了处理"大小未知"——
但那两个槽位描述的是**具体类型**（比如 `Sq`）的大小。
如果你从 `dyn` 里调一个返回 `Self` 的方法，
**返回值的类型在编译期完全未知**：编译器没法为它分配空间、没法生成代码。

**又是同一个理由：vtable 是固定布局，容不下"编译期未知的形状"。**

> 这就是为什么 `Clone` **不是** dyn-compatible：
> `fn clone(&self) -> Self`。想要 `dyn` 上的克隆，得手动写
> `fn clone_box(&self) -> Box<dyn Cloneable>`——把 `Self` 换成
> **大小已知**的 `Box<dyn ...>`。

### 7.2.4 三种分发方式的完整对照

```rust
pub fn area_generic<T: Shape>(s: &T) -> f64 { s.area() }   // 泛型
pub fn call_generic(s: &Sq) -> f64 { area_generic(s) }     // 调用点
pub fn area_dyn(s: &dyn Shape) -> f64 { s.area() }         // dyn
pub fn area_boxed(s: &Box<dyn Shape>) -> f64 { s.area() }  // Box<dyn>
```

```asm
; 泛型 + 内联：完全消失，只剩一次乘法
_call_generic:
	ldr	d0, [x0]
	fmul	d0, d0, d0
	ret

; dyn：从 vtable 偏移 24 取函数指针，尾跳转
_area_dyn:
	ldr	x1, [x1, #24]
	br	x1

; Box<dyn>：多一次解引用
_area_boxed:
	ldp	x8, x9, [x0]        ; x8 = 数据指针, x9 = vtable 指针
	ldr	x1, [x9, #24]
	mov	x0, x8
	br	x1
```

**这是这个最小调用点里能直接看到的动态分发成本**：

1. **一次额外的内存加载**：从 vtable 取地址（`ldr x1, [x9, #24]`）；
2. **一次间接跳转**：`br x1` —— 无法内联、分支预测器可能失手；
3. `Box<dyn>` 再多**一次解引用**（先取出胖指针）。

**注意：没有"虚函数表查找的开销"这种玄学**——
就是一条 `ldr` 加一条 `br`。剩下的代价都在"无法内联"上。

### 7.2.5 `#[unsafe(no_mangle)]` 不能加在泛型函数上

实测：

```text
warning: functions generic over types or consts must be mangled
   = note: `#[warn(no_mangle_generic_items)]` on by default
```

**这个 warning 本身就是"泛型 = 单态化"的证据**：
泛型函数的符号名必须编码实例化信息，否则 `area::<Sq>` 和 `area::<Circle>`
会撞名。而 `#[unsafe(no_mangle)]` 恰恰要求"不加编码"——
两者直接矛盾。

## 7.3 为什么必须这样设计

### 为什么 vtable 要"定长 + 固定偏移"？

因为调用点需要**一条指令取到地址**。如果槽位位置要动态计算，
每次调用都得先做一次查找——那就不是"一次 `ldr`"了。

**固定偏移换来的是 O(1) 的调用开销**，代价是"表的结构必须在编译期完全确定"。

### 为什么泛型方法不能"塞进 vtable 的某个槽"？

因为它**不是一个方法**，是**一族方法**（每个 `T` 一个）。
把"一族函数"塞进"一个槽位"，需要运行时的类型信息（像 Java 的泛型擦除 +
反射，或者 C++ 的模板 + 虚函数混合）。Rust 选择了不这么做——
**单态化是编译期的，`dyn` 是运行期的，两者不能混**。

如果你真的需要，可以手动分层：

```rust
pub trait Cache {
    fn get(&self, key: &str) -> Option<String>;
    fn insert_str(&mut self, key: &str, value: &str);   // 具体类型 → dyn 兼容
}
```

或者用 `dyn Any` + `downcast`（运行期类型检查，第 24 章会提到）。

### 为什么 `where Self: Sized` 能让某些方法"豁免"？

```rust
pub trait Cache {
    fn get(&self, key: &str) -> Option<String>;
    fn insert<T: ToString>(&mut self, key: &str, value: T) where Self: Sized;
    //                                                     ^^^^^^^^^^^^^^^^
}
```

`where Self: Sized` 的意思是"**这个方法只对具体类型可用，`dyn` 上不存在**"。
于是它**不需要进 vtable**——vtable 里没有它的槽位，
`dyn Cache` 也就调不到它。

**这正好从反面验证了 7.2.2 的论证**：
泛型方法的问题在于"要占 vtable 槽位但占不下"，
`where Self: Sized` 把它从 vtable 里**移出去**，问题就没了。

## 7.4 反直觉的点

### 反直觉之一：`dyn` 的代价不在"查找"，在"不能内联"

很多人以为"虚函数调用慢是因为要查表"。查表就是一条 `ldr`——
从 L1 缓存取一个指针，几个周期。

**真正的代价是 `br x1` 之后的代码无法被优化**：

- 无法内联 → 无法做常量传播、无法向量化、无法消除冗余；
- 间接跳转 → 分支预测器可能失手（尤其调用点有多个实现时）；
- 编译器无法知道会调到哪个函数 → 无法跨调用优化。

**所以 `dyn` 在"调用很重"的函数上几乎没有代价**，
在"调用很轻但调用频繁"的函数上代价很大。判据是**被调用函数的工作量**，
不是"调用了多少次"。

### 反直觉之二：`&dyn Trait` 是**胖指针**，`Box<dyn Trait>` 里那个 `Box` 也是

```rust
size_of::<&dyn Shape>()    == 16    // 数据指针 + vtable 指针
size_of::<Box<dyn Shape>>() == 16   // 同上
```

`&dyn Trait` 不是 8 字节，是 **16 字节**——这是"动态分发"在**内存**上的代价，
不只是 CPU 上的。放在 `Vec<Box<dyn Trait>>` 里，每个元素 16 字节。

**这也是为什么 `dyn` 不适合小对象**：一个 `Box<dyn Iterator<Item = u8>>`
光指针就 16 字节，而 `u8` 只有 1 字节。

### 反直觉之三：`dyn` 是 `!Sized`，但 `Box<dyn T>` 是 `Sized`

```rust
fn takes_dyn(x: dyn Shape);          // ❌ 编译不过：大小未知
fn takes_ref(x: &dyn Shape);         // ✅
fn takes_box(x: Box<dyn Shape>);     // ✅
```

`dyn Shape` 本身是 `?Sized` 的——**这正是 vtable 里要存 `size`/`align` 的原因**。
而 `&dyn Shape` 和 `Box<dyn Shape>` 都是 16 字节的**胖指针**，大小已知。

**注意 `Box<dyn T>` 是 `Sized` 这一点很重要**：
它让 `dyn` 能进 `Vec`、能当字段、能返回——**"未知大小"被 `Box` 包住了**。

### 反直觉之四：`area_dyn` 和 `dyn_area` 是同一个函数

实测：

```llvm
@area_dyn = unnamed_addr alias double (ptr, ptr), ptr @_RNvCsrtIYgyWToU_3lib8dyn_area
```

两个函数体完全相同，LLVM 把它们合并了。
**这和第 2 章"生命周期被擦除"、第 4 章"`Pin` 是零成本"是同一类现象**：
编译器在 LLVM IR 层面做等价性判断，把源码里"看起来不同"的东西合并掉。

## 7.5 亲手验证

```bash
tools/evidence.sh ch07-vtable
scripts/verify-all.sh ch07

# 看 vtable 的完整布局
sed -n '/__DATA,__const/,/literal8/p' .evidence/ch07-vtable-lib.O3.s

# 三种分发的对照
grep -A4 '^_call_generic:' .evidence/ch07-vtable-lib.O3.s
grep -A4 '^_area_dyn:'     .evidence/ch07-vtable-lib.O3.s
grep -A5 '^_area_boxed:'   .evidence/ch07-vtable-lib.O3.s

# 两个反例
rustc --edition 2024 --crate-type=lib \
      examples/ch07-vtable/fail/not_dyn_compatible_generic.rs
rustc --edition 2024 --crate-type=lib \
      examples/ch07-vtable/fail/not_dyn_compatible_self.rs
```

**怎么算验证成功**：

1. `.O3.s` 里能看到 `__DATA,__const` 段，`.asciz` 的下一行是 `.quad`
   —— vtable 的 24 字节头部 + 方法指针；
2. `dyn_area` 的汇编是 `ldr x1, [x1, #24]` + `br x1` ——
   这是该最小调用点的直接分发成本；
3. `call_generic` 的汇编是 `ldr` + `fmul` —— 泛型被完全内联；
4. 两个反例都报 **E0038**，且错误信息里都提到
   `needs to allow building a vtable`。

```bash
scripts/verify-all.sh ch07      # 13 条断言
```

## 7.6 与 unsafe 的关系

`dyn` 的 vtable 是编译器生成的，你不能（也不需要）手动构造。
但 **vtable 的存在本身**给了 `unsafe` 一些有意思的可能：

- 你可以用 `transmute` 把 `&dyn T` 拆成 `(data_ptr, vtable_ptr)`
  —— 但这依赖 vtable 的具体布局，**没有稳定性保证**；
- 更常见的做法是用 `#[repr(C)]` 自己定义"手动 vtable"结构
  （很多 FFI 和插件系统这么做），这时**布局是你控制的**，
  但也意味着**健全性由你保证**。

**不要依赖编译器生成的 vtable 布局**——它是实现细节，
Rust 没有承诺它稳定。要看可以，要依赖不行。

## 7.7 小结

- **`dyn` 的直接成本可以观察**：本章最小样本是一条额外 `ldr` 加一次
  间接 `br`；真实成本还包括可能失去的内联和后续优化机会。
- **`&dyn T` 是 16 字节的胖指针**，不是 8 字节——
  这是动态分发的**内存**代价。
- **dyn compatibility（旧称 object safety）的判据是"能不能建出定长 vtable"**：
  - 泛型方法：实例数量不定 → 表长不定 → ❌；
  - 返回 `Self`：`Self: ?Sized`，返回值形状未知 → ❌；
  - `where Self: Sized`：**把方法移出 vtable** → ✅。
- **`Clone` 不是 dyn-compatible**，因为 `fn clone(&self) -> Self`。
  想克隆 `dyn`，得手写 `fn clone_box(&self) -> Box<dyn Trait>`。
- **`#[unsafe(no_mangle)]` 不能加在泛型函数上**——
  warning 本身就是"泛型 = 单态化"的证据。
- **`dyn` 的真正代价在"不能内联"**，所以它适合"调用很重"的函数，
  不适合"调用轻但频繁"的函数。

下一章我们从"类型"转向"实现"：**coherence 和孤儿规则**——
为什么你不能给别人的类型实现别人的 trait。
