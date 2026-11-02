# 第 30 章：安全 `ArrayVec<T, N>` - 实测证据

> 最后验证：**rustc 1.99.0** / LLVM 23.1.1 / `aarch64-apple-darwin`

## 不变式

```text
len <= N
data[..len]  = 已初始化、有效、由容器拥有的 T
data[len..]  = 不得按 T 读取或析构
```

每个 unsafe block 都只做一件能从这三条直接推出的事：构造 slice、读出单个元素、
移动重叠区间，或析构已初始化区间。

## 测试矩阵

```bash
cargo test -p ch30-arrayvec
scripts/verify-miri.sh
```

| 用例 | 验证内容 |
|---|---|
| push/pop/insert/remove | 顺序与 len 一致 |
| full | 满容量时把值还给调用者，不泄漏所有权 |
| drop accounting | 每个值恰好析构一次 |
| partial construction | 初始化函数报错时只清理前缀 |
| panicking construction | 初始化函数 panic 时也只清理前缀 |
| partial IntoIter | iterator 被提前丢弃时清理未消费后缀 |
| `N=0` / ZST | 零容量和零大小类型不破坏指针不变式 |
| `Clone` panic | 只清理已完成的克隆，源容器保持不变 |
| iterator panic / 满容量 | 保留已插入前缀，满容量返回第一个未插入值 |
| `retain_mut` predicate panic | 压紧保留元素与未处理后缀，恢复有效前缀 |
| `retain_mut` 中元素析构 panic | 先转移清理责任，guard 不重复析构该元素 |
| 带 Drop 的 ZST / forget | 按逻辑元素计数，忘掉 iterator 不重用未消费区域 |

`tests/arrayvec_ub.rs` 是对应的负向探针：先发布 len、再执行会 panic 的用户
构造函数。unwind 时析构器读取未初始化 bool，Miri 必须给出 Undefined Behavior。
普通 `cargo test` 不执行这个目标的 UB 路径，`scripts/verify-miri.sh` 对它作
“必须报 UB”的断言。`retain_mut` 每个 unsafe block 的区域证明见正文 30.9.2。

## 编译器证据

```bash
tools/evidence.sh ch30-arrayvec
```

- MIR 中可见 `MaybeUninit::write` 和 `assume_init_read`；
- `Drop` 只把 `data[..len]` 变成 slice 后调用 `drop_in_place`；
- `sum_arrayvec` 没有 `__rust_alloc`，容量完全在栈上；
- 当前优化器**没有**把包装完全消掉：裸数组版本使用 SIMD，`ArrayVec` 版本仍有
  48 字节栈帧和四次标量加法。这证明“无堆分配”不等于“与裸数组指令相同”。

Miri 通过只能证明测试覆盖的执行路径没有触犯当前模型，不能单独证明泛型容器对
所有 `T`、`N` 和调用序列都 sound。不变式论证仍然是主证据。
