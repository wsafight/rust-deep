# 第 5 章：边界检查 —— 安全检查什么时候是免费的

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`

## 复现命令

```bash
tools/evidence.sh ch05-bounds
scripts/verify-all.sh ch05
```

## ★ 核心结论：热路径一样，**失败路径不一样**

`-O` 的 AArch64 原始输出：

```asm
; safe —— v[i]
_safe:
	cmp	x2, x1
	b.hs	LBB3_2
	ldrb	w0, [x0, x2]        ; ← 热路径 3 条指令
	ret
LBB3_2:                          ; ← 失败路径
	bl	__RNvNtCs8Mbv00yxnRz_4core9panicking18panic_bounds_check

; checked_unchecked —— assert!(i < v.len()) + get_unchecked
_checked_unchecked:
	cmp	x2, x1
	b.hs	LBB0_2
	ldrb	w0, [x0, x2]        ; ← 热路径**逐条相同**
	ret
LBB0_2:
	mov	w1, #29
	bl	__RNvNt...panicking5panic   ; ← 但失败路径是 panic（assertion 消息）

; raw_unchecked —— 直接 get_unchecked，没有 assert
_raw_unchecked:
	ldrb	w0, [x0, x2]        ; ← 检查**彻底消失**
	ret
```

### 三层讲法

**(1) `get_unchecked` 不是"跳过检查"，是"优化提示的语法糖"。**
你手动写了 `assert`，编译器就把它变成同样的比较+分支 —— 生成的代码与 `v[i]` **相同**。

**(2) 但两者并不"完全相同"** —— 失败时的行为不同：
- `safe`：`panic_bounds_check`，报 **索引和长度**；
- `checked_unchecked`：`panic`，报 **assertion 消息**。
文件大小、panic 消息、`#[track_caller]` 的位置都不一样。
→ **写"完全一致"是过强的结论**，本书必须写清这个区别。

**(3) 真正危险的是 `raw_unchecked`** —— 删掉 assert 却仍假设成立，**那才是 UB**。
热路径看起来一样快，但**没有任何东西保证 `i < len`**。

## 检查会被自动消除的情形

```asm
; provably_in_bounds —— v[i % 4]，v: &[u8; 4]
_provably_in_bounds:
	and	x8, x1, #0x3       ; ← 取模
	ldrb	w0, [x0, x8]       ; ← 没有边界检查！
	ret
```
LLVM 能从 `i % 4` 的范围推出 `i < 4`，于是**检查被证明多余而删除**。
→ **最好的优化不是"去掉检查"，是"让编译器证明检查多余"**：
用 `% N`、`& (N-1)`、或 `get` + `match` 表达约束，比 `unsafe` 更安全也更清晰。

## O0 vs O3 的对照（第 3.5 节的教训）

| 例子 | O0 | O3 |
|---|---|---|
| `ch01-borrow` 的 `simple`（`Point`） | 23 行 | 2 行 |
| `ch05-bounds` 的 `sum_all` | 见 `.evidence/` | 向量化 |

⚠️ **引用行数必须写明是哪个 example**，避免不同证据文件的行号混淆。

## 待办

- [ ] 加断言：`safe` 与 `checked_unchecked` 的**热路径指令序列相同**（逐条比对）
- [ ] 加断言：`raw_unchecked` **不含** `cmp`/`b.hs`
- [ ] 加断言：`provably_in_bounds` **不含**边界检查
- [ ] 接 criterion 做真实基准 ——
      **在拿到数据之前，正文不要写"更快/更慢"**
