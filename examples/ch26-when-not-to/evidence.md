# 第 26 章：何时**不该**用 `unsafe` — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch26-when-not-to
scripts/verify-all.sh ch26      # 10 条断言
```

## 关键结论与断言（10 条，全绿）

| # | 结论 | 断言 |
|---|---|---|
| 1 | ★ 循环内索引：unsafe 版本被折叠成 alias | `.O3.s` 里 `^_sum_unchecked = _sum_safe` |
| 2 | ★ 掩码索引：同样是同一个函数 | `.O3.s` 里 `^_masked_unchecked = _masked_safe` |
| 3 | `masked_safe` 检查被消除 | 函数体内 `and x?, x?, #0x3` |
| 4 | `masked_safe` 没有 panic 路径 | 函数体内**无** `panic_bounds_check` |
| 5 | 索引来自外部 → 检查必须保留 | `get_safe` 函数体内有 `panic_bounds_check` |
| 6 | `get_unchecked` 没有检查 | 函数体内**无** `panic_bounds_check` |
| 7 | `safe_double_add` 敢合并两次加法 | 函数体内 `add w8, w9, w8, lsl #1` |
| 8 | `raw_double_add` 必须重读 | 函数体内 `ldr w9, [x1]` |
| 9 | `split_at_mut` 是安全替代 | `.O3.s` 里 `^_split_and_sum:` |
| 10 | `swap_two` 保留检查 | 函数体内有 `panic_bounds_check` |

## ★ 核心对照表（全部实测，`-O` / AArch64）

| 情形 | 安全版本 | `unsafe` 版本 | 差值 |
|---|---|---|---|
| 循环内索引 `0..v.len()` | **39** 条指令（含向量化） | **同一个函数** | **0** |
| 掩码索引 `i & 3`（长度 4） | **3** 条指令 | **同一个函数** | **0** |
| 外部传入的索引 | **11** 条 + panic 路径 | **2** 条指令 | **9** 条 |

### 第一组：循环内索引

```asm
_sum_unchecked = _sum_safe
```

**alias** —— LLVM 判定两者逐位等价。
原因：循环上界就是 `v.len()`，编译器证得出 `i < v.len()`，
安全版本里的检查**早已被消除**。

（迭代器版本 `sum_iter` 也是 39 条指令，与 `sum_safe` 逐字节相同。
**三种写法，一个函数。**）

### 第二组：掩码索引（最干净的例子）

```asm
_masked_safe:
	and	x8, x1, #0x3
	ldr	x0, [x0, x8, lsl #3]
	ret
```

**3 条指令，没有任何检查。** LLVM 从 `i & 3` 直接推出 `i & 3 < 4`。

```asm
_masked_unchecked = _masked_safe
```

**同一个函数。** 这个例子最适合用来建立直觉：
"安全代码慢"这个印象，很多时候来自**没看汇编**。

（第 5 章实测过同一件事：`provably_in_bounds` 只剩一条 `and x?, x?, #0x3`。）

### 第三组：外部传入的索引 —— 唯一有差值的情形

```asm
_get_safe:
	cmp	x2, x1                 ; ← 边界检查
	b.hs	LBB0_2
	ldr	x0, [x0, x2, lsl #3]
	ret
LBB0_2:                          ; cold 路径
	...  bl  panic_bounds_check
```

```asm
_get_unchecked:
	ldr	x0, [x0, x2, lsl #3]
	ret
```

**省下 `cmp` + `b.hs` 两条指令**（加上一段本来也不会执行的 cold 代码）。

★ 这是本章唯一一个 `unsafe` 真的省下东西的情形。

## ★ 反例：`unsafe` 反而**更慢**（第 24 章的结论）

```asm
_safe_double_add:              ; 5 条指令
	ldr	w8, [x1]
	ldr	w9, [x0]
	add	w8, w9, w8, lsl #1     ; ★ 两次加法合并
	str	w8, [x0]
	ret

_raw_double_add:               ; 8 条指令
	ldr	w8, [x1]
	ldr	w9, [x0]
	add	w8, w9, w8
	str	w8, [x0]
	ldr	w9, [x1]               ; ★ 必须重新读（没有 noalias）
	add	w8, w9, w8
	str	w8, [x0]
	ret
```

**`unsafe` 不是"解锁更多优化"，而是"放弃优化"** ——
而且这份损失**不报错、不警告、不出现在 profiler 里**。

## ★ 安全替代：标准库已经解决了最经典的两个场景

| 你想做的事 | 手写需要 `unsafe` 因为… | 安全替代 |
|---|---|---|
| 把一个切片劈成两半 | 裸指针 + 偏移 | **`split_at_mut`** |
| 按索引取两个不同的元素 | 两个 `&mut` 指向同一数组 | **`split_at_mut`** |

```rust
pub fn swap_two(v: &mut [u64], i: usize, j: usize) -> u64 {
    if i != j && i < v.len() && j < v.len() {
        let (lo, hi) = if i < j { (i, j) } else { (j, i) };
        let (left, rest) = v.split_at_mut(hi);   // ← 安全地拿到两个不重叠的 &mut
        std::mem::swap(&mut left[lo], &mut rest[0]);
    }
    v.first().copied().unwrap_or(0)
}
```

★ 注意它的汇编里**有 `panic_bounds_check`** —— 因为 `i`、`j` 是外部输入。
**这正是安全代码该有的样子**：有风险的地方检查，没风险的地方（掩码索引）不检查。

## ★ 判据：先读汇编，再谈 `unsafe`

```text
先读汇编（快、确定、不需要构造负载）
   │
   ├── 同一个函数？ → 停。没有 unsafe 的必要。
   │
   └── 真的不同？ → 再问：这个差值重要吗？→ 再 benchmark
```

**为什么比"先 benchmark"更好**：benchmark 回答"哪个快"，
而这里要先回答的是"**有没有区别**"。
如果两个版本生成同一个函数，benchmark 必然是"没有区别"
—— **而你已经为此付出了一段 `unsafe`。**

### ⚠️ 一个重要提醒

边界检查的消除**不是承诺**。编译器可能因为版本变化、内联决策、
或周围代码的微小改动而**不再消除**某个检查。

所以：

- **不要依赖"检查会被消除"来写安全代码**；
- 更**不要**因为"我测过它被消除了"就去写 `unsafe` ——
  那是用编译器的**实现细节**当自己的**安全前提**。

## 完整判断链（四步）

```text
汇编：有没有区别？
   ↓ 有
Miri：我的 unsafe 违反别名规则吗？（第 25 章）
   ↓ 没有
论证：SAFETY 注释能说服别人吗？（第 24 章）
   ↓ 能
benchmark：这个差值对整体有影响吗？
```

**四步都过了，`unsafe` 才是值得的。**

## 把 `unsafe` 收进边界（推荐模式）

```rust
// ❌ 大 unsafe 块：义务范围模糊
pub unsafe fn f(v: &[u64], i: usize) -> u64 {
    unsafe {
        let x = *v.get_unchecked(i);
        let y = *v.get_unchecked(i + 1);   // 这个的边界呢？
        x + y
    }
}

// ✅ 小 unsafe 块 + 明确前提（接口是安全的）
pub fn f(v: &[u64], i: usize) -> u64 {
    assert!(i + 1 < v.len(), "需要至少两个元素");   // ← 前提检查
    // SAFETY: 上面的 assert 保证 i 和 i + 1 都在界内
    unsafe { *v.get_unchecked(i) + *v.get_unchecked(i + 1) }
}
```

★ **代价恰好就是你想省掉的那两条指令**，换来的是"失败时 panic 而不是 UB"。
值得想一想。

## 交叉验证（可选）

```bash
grep '^_sum_unchecked = \|^_masked_unchecked = ' .evidence/ch26-when-not-to-lib.O3.s
for f in sum_safe sum_unchecked get_safe get_unchecked masked_safe safe_double_add raw_double_add; do
  n=$(awk -v s="^_$f:" '$0~s{on=1} on{print} on&&/cfi_endproc/{exit}' \
      .evidence/ch26-when-not-to-lib.O3.s | grep -cE '^\s+[a-z]')
  printf "%-20s %s\n" "$f" "$n"
done
```

## 待办

- [x] 10 条断言全绿（`verify-all.sh ch26`）
- [x] 三组对照落库（两组 alias、一组有差值）
- [x] 复用了第 24 章的 `safe_double_add` / `raw_double_add` 作为"更慢"的反例
- [ ] 若将来接 criterion，可以对第三组（外部索引）做真实的 benchmark ——
      **在拿到数据之前正文不写"更快/更慢"的具体倍数**

## 全书收尾

至此 **1–26 章正文全部完成**。证据链：

| 层 | 章节 |
|---|---|
| MIR | 1、2、4、9、14、15、19、20、25（澄清） |
| LLVM IR | 12、24、26 |
| 汇编 / `.data` 段 | 5、7、11、13、16、17、18、19、26 |
| Miri | 19、25 |
| 类型系统（编译/不编译） | 3、6、8、10、20、21–23 |
