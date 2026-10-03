# 第 27 章：`Drop`、drop flags 与 panic safety - 实测证据

> 最后验证：**rustc 1.99.0** / LLVM 23.1.1 / `aarch64-apple-darwin`

## 结论与证据

| 结论 | 证据 | 命令 |
|---|---|---|
| 局部变量逆序析构 | 运行期测试得到 `[2, 1]` | `cargo test -p ch27-drop` |
| 字段按声明顺序析构 | 运行期测试得到 `[1, 2]` | `cargo test -p ch27-drop` |
| 条件移动不会重复析构 | 两条分支都只记一次 drop | `cargo test -p ch27-drop` |
| panic 展开执行 cleanup | `catch_unwind` 后计数为 1 | `cargo test -p ch27-drop` |
| `Drop` 类型不能移出字段 | E0509 | `scripts/verify-evidence.sh ch27` |
| drop check 保证析构时借用仍有效 | E0597 | `scripts/verify-evidence.sh ch27` |

## drop flag 的 MIR 形态

```bash
tools/mir.sh ch27-drop
sed -n '/fn conditional_move/,/^}/p' .evidence/ch27-drop-lib.mir
```

`value` 是否仍需析构由一个布尔局部变量记录。移动到 `consume` 前该位被清除；
正常返回和 unwind cleanup 都先检查它，再决定是否执行 `drop`。这不是额外的
语言对象，而是编译器为了实现“每个已初始化值恰好析构一次”生成的控制流状态。

## panic safety 的边界

```bash
sed -n '/fn unwind_cleanup/,/^}/p' .evidence/ch27-drop-lib.mir
```

正常路径会析构 `_guard`；`panic_fmt` 的 unwind edge 也进入析构 `_guard` 的
cleanup block，然后 `resume`。因此 RAII 能保证锁、文件和临时缓冲区被释放。

它不能自动恢复业务不变式：如果程序先修改字段 A、随后在修改字段 B 前 panic，
析构器只能清理资源，不能猜出 A 应该回滚。需要事务、临时对象或 guard 显式维护
“提交前不可见”的状态。

`panic=abort` 是另一条语义：进程直接终止，不运行展开 cleanup。因此不能把
析构器当作必须发生的持久化动作；`mem::forget` 是安全 API 也证明了 Rust 允许
析构器永远不运行。

## 反例

```text
error[E0509]: cannot move out of type `Envelope`, which implements the `Drop` trait
error[E0597]: `value` does not live long enough
```

E0509 防止 `Drop::drop(&mut self)` 看到一个字段已被搬空的半残对象。需要取出字段
时，优先把字段设计成 `Option<T>` 并调用 `take()`，正如第 17 章线程池关闭发送端。

E0597 是 drop check：`Inspector::drop` 仍可能读取内部引用，所以被借用的值必须
活到 `Inspector` 析构之后。
