# 第 24 章：unsafe 的边界哲学 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch24-noalias
scripts/verify-all.sh ch24      # 9 条断言
```

## 关键结论与断言（9 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | `&mut` 只读时与 `&` 的签名**完全一致** | `sum_mut_ro` 行含 `noalias.*readonly` |
| 2 | `&mut` 真的写了，`readonly` 消失 | `sum_mut_rw` 行**不含** `readonly` |
| 3 | LLVM 把两个函数**合并成一个** | `@sum_shared = ... alias ... ptr @sum_mut_ro` |
| 4 | 两个 `&u64` 参数**都**标 `noalias` | `two_shared` 行含 `noalias` ×2 |
| 5 | `noalias` 让跨参数 SIMD 成为可能 | `add_all` 的汇编含 `add.2d` |
| 6 | ★ 安全引用参数带 `noalias` | `@safe_double_add(ptr noalias` |
| 7 | ★ 裸指针参数**不**带 `noalias` | `@raw_double_add(ptr` 后无 `noalias` |
| 8 | ★ 安全版本敢合并两次加法 | 汇编含 `add w8, w9, w8, lsl #1` |
| 9 | ★ 裸指针版本必须重新读 `*b` | `raw_double_add` 函数体内含 `ldr w9, [x1]` |

## 证据 1：`&mut` 不读时，与 `&` 的契约完全相同

```llvm
; sum_mut_ro —— &mut [u64]，但函数体只读
define noundef i64 @sum_mut_ro(ptr noalias nofree noundef nonnull readonly align 8 captures(none) %v.0, ...)

; sum_mut_rw —— 真的写了 v[0] = 7
define noundef i64 @sum_mut_rw(ptr noalias nofree noundef nonnull           align 8 captures(none) %v.0, ...)
```

**讲法**：`&mut` 带来的**不是"readonly 的缺失"**，而是 `noalias` +
"允许写入"的许可。`readonly` 是 LLVM 看**函数体**推出来的，与 `&` / `&mut` 无关。
→ **"`&mut` 更慢"是错的。**

## 证据 2：★ LLVM 直接把两个函数合并了

```llvm
@sum_shared = unnamed_addr alias i64 (ptr, i64), ptr @sum_mut_ro
```

LLVM 判定 `sum_shared` 与 `sum_mut_ro` **逐位等价**，于是前者变成后者的 alias。
这比"并排贴两段一样的汇编"更有说服力：**不是"看起来一样"，是编译器认定同一个函数。**

## 证据 3：★ `&T` 也是 `noalias`，而这是**健全**的

```llvm
define noundef i64 @two_shared(ptr noalias ... readonly ... %a,
                               ptr noalias ... readonly ... %b)
```

两个 `&u64` 参数**都**被标 `noalias`，而 `&x, &x` 指向同一块内存在安全 Rust 里合法。
为什么不是 UB？因为 LLVM LangRef 对 `noalias` 的定义是：

> "This guarantee only holds for memory locations that are **modified**,
>  by any means, during the execution of the function."

即 **`noalias` 只约束"被修改"的内存**。两个只读指针别名同一处 —— 没有内存被修改 ——
不触发任何义务。

**实测验证**（在 `ch25-aliasing` 的 Miri 用例里，见 `scripts/verify-miri.sh`）：
```rust
let v = 21u64;
let p: *const u64 = &v;
let (a, b) = unsafe { (&*p, &*p) };      // 两个共享借用指向同一处
assert_eq!(a.wrapping_add(*b), 42);      // Miri 下通过，不报 UB
```
→ **书里必须引这句 LangRef 原文。** `&T: noalias` 是正确的，不是近似。

⚠️ 注意这条证据**不在本章的 example 里**，而在 `ch25-aliasing/tests/sb_legal.rs` ——
因为它需要 Miri 才能给出判定（普通运行下这个断言无论如何都会过）。

## 证据 4：★ `noalias` 真正的威力在跨参数时才显现

`add_all(dst: &mut [u64], a: &[u64], b: &[u64])` 的 `-O` AArch64：

```asm
	ldp	q0, q1, [x9, #-32]
	ldp	q2, q3, [x9], #64
	ldp	q4, q5, [x10, #-32]
	ldp	q6, q7, [x10], #64
	add.2d	v0, v4, v0        ; ★ 三路独立 SIMD 加法 + 4 路展开
	add.2d	v1, v5, v1
	add.2d	v2, v6, v2
	add.2d	v3, v7, v3
	stp	q0, q1, [x11, #-32]
	stp	q2, q3, [x11], #64
```

LLVM 敢同时读 `a`、读 `b`、写 `dst` 而不做任何别名检查，
**唯一依据就是三个参数的 `noalias`**。这是 `noalias` 从"元数据"变成"实际性能"的画面。

对照 `two_shared`（两个只读参数）—— 汇编里**没有任何别名检查**：
```asm
_two_shared:
	ldr	x8, [x0]
	ldr	x9, [x1]
	add	x0, x9, x8
	ret
```

## 证据 5：`captures` —— 比 `noalias` 更细的 Rust 特有约束

实测差异：
- `&mut [u64]`（写）：`captures(none)`
- `&[u64]` 经 `black_box` 传递：出现过 `captures(address)`

`captures` 描述"callee 能把指针捕获到哪一步"（`address` / `provenance` / `read_provenance` …），
是 Rust 特有的、比 `noalias` 更细的约束。**值得单独一节讲。**

## ★ 证据 6：同一段逻辑 —— 安全引用 5 条指令，裸指针 9 条

```rust
pub fn safe_double_add(a: &mut i32, b: &i32) {
    let x = *b; *a += x; *a += *b;
}

/// # Safety: 与 safe_double_add 相同（a 与 b 不重叠）
pub unsafe fn raw_double_add(a: *mut i32, b: *const i32) {
    unsafe { let x = *b; *a += x; *a += *b; }
}
```

IR 签名（差别只有 `noalias`）：

```llvm
define void @safe_double_add(ptr noalias nofree noundef align 4 captures(none) ... %a,
                             ptr noalias nofree noundef readonly align 4 captures(none) ... %b)

define void @raw_double_add(ptr noundef captures(none) %a,
                            ptr noundef readonly captures(none) %b)
```

`-O` AArch64：

```asm
_safe_double_add:              ; 5 条指令
	ldr	w8, [x1]               ; 读 *b
	ldr	w9, [x0]               ; 读 *a
	add	w8, w9, w8, lsl #1     ; ★ 两次加法合并成 *a + *b*2
	str	w8, [x0]
	ret

_raw_double_add:               ; 9 条指令
	ldr	w8, [x1]
	ldr	w9, [x0]
	add	w8, w9, w8
	str	w8, [x0]
	ldr	w9, [x1]               ; ★ 必须重新读 *b（可能被上面的 store 改了）
	add	w8, w9, w8
	str	w8, [x0]
	ret
```

**讲法**：`unsafe` 版本不是"更快的版本"，而是**主动放弃了编译器的一项优化**。
LLVM 不是"不优化裸指针"，而是**没有依据去优化** ——
`noalias` 是 Rust 送给 LLVM 的信息，裸指针不携带它。

★ 这份损失**不报错、不警告、不出现在 profiler 里**。
这就是"为了绕过借用检查器而写 `unsafe`"的真实代价。

## 本章与 unsafe 的关系（正文要点）

- `unsafe` 的**唯一义务**：维持这些元数据的**前提**。
- 一旦 `unsafe` 撒谎（比如让两个 `&mut` 指向同一处），
  就不是"慢一点"，而是 **UB**——因为 LLVM 已经按 `noalias` 优化过了。
- **健全性论证的模板**：指出你依赖哪条契约（`noalias` / `readonly` /
  `dereferenceable` / `captures`），再论证你的 `unsafe` 代码为什么没有破坏它。
