# 第 7 章：dyn vs 泛型 —— vtable 布局与间接调用 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> 本机工具链：`rustc 1.98.1` / `cargo 1.98.1` / LLVM 22.1.8。

## 复现命令

```bash
tools/evidence.sh ch07-vtable        # 生成 .s / .ll / .mir / .o
scripts/verify-all.sh ch07           # 跑本章 6 条断言
tools/objdump.sh ch07-vtable 3       # 需要反汇编时
```

## 关键结论与断言（6 条，全绿）

| # | 结论 | 断言（`scripts/verify-all.sh`） |
|---|---|---|
| 1 | 第一个方法在 vtable 偏移 **24** | `ldr x?, [x?, #24]` |
| 2 | 第二个方法在偏移 **32**（按声明顺序） | `ldr x?, [x?, #32]` |
| 3 | 调用是间接尾跳转 | `^\s*br\s+x?` |
| 4 | vtable 前 24 字节含 size=8 / align=8 | `.asciz "\0…\b…\b…"` |
| 5 | vtable 里真的有 `area` 的函数指针 | `.quad __RNvX…Shape4area` |
| 6 | vtable 里真的有 `name` 的函数指针 | `.quad __RNvX…Shape4name` |

## 原始输出（`.evidence/ch07-vtable-lib.O3.s`）

### 1) 间接调用：偏移 24 + 尾跳转

```asm
__RNvCsrtIYgyWToU_3lib8dyn_area:
	ldr	x1, [x1, #24]     ; vtable 第 4 个 slot（前 3 个是 drop/size/align）
	br	x1                ; 尾调用：不是 bl，因为没有后续工作
```

### 2) 两个方法、按声明顺序（`dyn_both`）

```asm
	ldr	x8, [x1, #24]     ; area
	blr	x8
	ldr	x8, [x19, #32]    ; name
	blr	x8
```

### 3) vtable 本体（`.data` 段，`--emit=asm` 就能看到，不需要 objdump）

```asm
	.section	__DATA,__const
	.p2align	3, 0x0
l_anon.684219fb9108ceced0d7d3641a5368d1.0:
	.asciz	"\000\000\000\000\000\000\000\000\b\000\000\000\000\000\000\000\b\000\000\000\000\000\000"
	.quad	__RNvXCsrtIYgyWToU_3libNtB2_2SqNtB2_5Shape4area
	.quad	__RNvXCsrtIYgyWToU_3libNtB2_2SqNtB2_5Shape4name
```

**逐字节读法**：
- 前 24 字节（3 slot）= `drop_in_place`（此处 `\0`×8）、`size = 8`、`align = 8`
  （`.asciz` 里的 `\b` 就是 8）—— 与 `Sq(f64)` 精确吻合；
- 偏移 **24** = `area`，偏移 **32** = `name`，与上面的 `ldr … #24` / `#32` 一一对应。

### 4) 静态 vtable 与字符串字面量

```asm
	.globl	__RNvCsrtIYgyWToU_3lib9STATIC_VT
__RNvCsrtIYgyWToU_3lib9STATIC_VT:
	.quad	l_anon.684219fb9108ceced0d7d3641a5368d1.1   ; 数据指针（指向 Sq(2.0)）
	.quad	l_anon.684219fb9108ceced0d7d3641a5368d1.0   ; vtable 指针
```
`&dyn Trait` 是**胖指针**：两个机器字，`[数据指针, vtable 指针]`。
这里 `.quad` 的顺序就是胖指针的内存布局。

## ★ 三种分发方式的对照（同一份逻辑）

```rust
pub fn area_generic<T: Shape>(s: &T) -> f64 { s.area() }   // 泛型（单态化）
pub fn call_generic(s: &Sq) -> f64 { area_generic(s) }     // 调用点
pub fn area_dyn(s: &dyn Shape) -> f64 { s.area() }         // dyn（vtable）
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

; Box<dyn>：多一次解引用（先取出胖指针，再走 vtable）
_area_boxed:
	ldp	x8, x9, [x0]        ; x8 = 数据指针, x9 = vtable 指针
	ldr	x1, [x9, #24]
	mov	x0, x8
	br	x1
```

**这就是动态分发的全部代价**：一次额外的内存加载（从 vtable 取地址）+ 一次间接跳转
（无法内联、无法预测分支）。`Box<dyn>` 再多一次解引用。

★ **`area_generic` 不能加 `#[unsafe(no_mangle)]`** ——
泛型函数必须单态化，符号名必然带实例化信息。加了会得到
`warning: functions generic over types or consts must be mangled`。
**这个 warning 本身就是"泛型 = 单态化"的证据。**

★ 实测还发现：`_area_dyn = __RNvCsrtIYgyWToU_3lib8dyn_area`
—— `area_dyn` 和 `dyn_area` 函数体相同，被 LLVM 折叠成了同一个函数。

## 前置条件（容易踩的坑）

**必须有 unsize 强制转换的构造点，vtable 才会生成。**
只在函数签名里写 `&dyn Shape` **不会**生成 vtable——
实测初版 `examples/ch07-vtable` 只有 `dyn_area(&dyn Shape)`，
`.O3.s` 里 `__const`/`asciz` 出现 **0 次**，vtable 根本不存在。

现在 `examples/ch07-vtable/src/lib.rs` 里有三个构造点：
`make_dyn(v) -> Box<dyn Shape>`、`static STATIC_VT: &(dyn Shape + Sync)`、
`dyn_both(&dyn Shape)`（同时调两个方法）。

附带的一课：`static` 要求 `Sync`，所以必须写 `&(dyn Shape + Sync)`——
`Send`/`Sync` 是**类型层**的检查，编译器直接拒绝 `&dyn Shape` 进 `static`。

## 交叉验证（可选）

```bash
tools/objdump.sh ch07-vtable 3
# 或直接看 __const 段的原始字节：
llvm-objdump -s -j __const .evidence/ch07-vtable-lib.O3.o
llvm-objdump -r .evidence/ch07-vtable-lib.O3.o     # 看 .quad 指向的符号
```
