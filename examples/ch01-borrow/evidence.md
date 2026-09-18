# 第 1 章：借用检查器到底在检查什么 — 实测证据

> 最后验证：**rustc 1.98.1**（2026-09-01）/ LLVM 22.1.8 / `aarch64-apple-darwin`
> 本机工具链：`rustc 1.98.1` / `cargo 1.98.1` / LLVM 22.1.8。

## 复现命令

```bash
tools/evidence.sh ch01-borrow
rustc --edition 2024 --crate-type=lib examples/ch01-borrow/fail/E0502.rs   # 反例
```

## 证据 1：最小可读 MIR 样本（`simple`）

`examples/ch01-borrow/src/lib.rs` 的 `simple()` —— **只有 29 行 MIR、单个基本块**。
这是本章的核心教学材料：读者应该能一眼读完，把注意力放在"借用的存活区间"上。

```
fn simple() -> f64 {
    let mut _0: f64;
    let mut _1: Point;
    let mut _4: f64;
    scope 1 {
        debug p => _1;
        let _2: &Point;          // ← 借用是"一个具名的临时变量"
        scope 2 {
            debug r => _2;
            let _3: f64;
            scope 3 {
                debug v => _3;
            }
        }
    }

    bb0: {
        _1 = Point { x: const 1f64, y: const 2f64 };
        _2 = &_1;                 // ← 借用在这里"产生"
        _3 = copy ((*_2).0: f64); // ← 最后一次使用 _2
        (_1.0: f64) = const 3f64; // ← 写 p.x：借用已死，合法
        _4 = copy (_1.1: f64);
        _0 = Add(copy _3, move _4);
        return;
    }
}
```

**讲法**：借用检查器不"理解"你的语义，它检查的是**控制流图上的数据流事实**：
`_2 = &_1` 之后到 `_2` 最后一次被使用之前，`_1` 不能有冲突访问。
把 `p.x = 3.0` 挪到 `let v = r.x` **之前**，同一份代码就报 E0502 ——
**变的是使用点的位置，不是词法作用域**。这就是 NLL。

## 证据 2：`nll_ok` —— 借用与可变借用交错（最干净的 NLL 演示）

```
bb0: {
    _1 = Counter { n: const 0_u64 };
    _2 = &(_1.0: u64);                 // ← ① 共享借用产生
    _3 = copy (*_2);                   // ← ② 最后一次使用 _2
    _5 = &mut _1;                      // ← ③ 可变借用：合法
    _4 = Counter::bump(move _5) -> ...;
}
```

把 `let seen = *r;` 删掉，同一份代码立刻报 E0502。
**变的是使用点的位置，词法作用域一点没动。**

## 证据 3：`branch_all_use` —— 路径敏感

`if` / `else` **两个分支都**使用 `first`，`if` 之后的 `v.push(4)` 合法。
对照 `fail/use_after_conflict.rs`：**只在一个分支里**使用，就报 E0502 ——
因为"不走 if"那条路径仍然能到达使用点。

## 证据 4：反例的错误信息结构（`fail/E0502.rs`）

```bash
$ rustc --edition 2024 --crate-type=lib examples/ch01-borrow/fail/E0502.rs
error[E0502]: cannot borrow `v` as mutable because it is also borrowed as immutable
  --> examples/ch01-borrow/fail/E0502.rs:23:5
   |
22 |     let first = &v[0];     // 不可变借用产生
   |                  - immutable borrow occurs here
23 |     v.push(4);             // 可变借用：冲突
   |                  ^^^^^^^^^ mutable borrow occurs here
24 |     println!("{first}");   // 不可变借用在这里才最后一次被使用 → E0502
   |                ----- immutable borrow later used here
```

**讲法**：错误信息的三段标注**就是借用检查器眼中的三个关键点**——
借用的**产生点**、**冲突点**、以及**最后一次使用点**。
第三点正是 NLL 的核心：如果没有第三行（`println!`），这段代码能编译。

## 证据 5：`--emit=mir` 的产物是"借用检查之后"的

**实测**：把上面那段失败的程序喂给 `--emit=mir`，报 E0502 之后
**`.mir` 文件根本不存在**（`ls: p2.mir: No such file or directory`）。

→ 这是本章方法论的关键一环：`--emit=mir` 输出的是**借用检查通过之后**的产物，
正是我们想要的"借用检查器眼中的世界"。**读者零额外安装**（stable 即可）。

## 附：本 example 的三个小设计

1. **反例单独放 `fail/` 目录**（不进 `src/`），因为：
   - 它编译不过，会拖垮 `cargo build`；
   - 即使用 `vec!` 之外的写法，`Vec` 的展开（`Box::new_uninit` / `box_assume_init_into_vec_unsafe`
     / 对齐与空指针断言）会把 MIR 淹没成 100+ 行，教学价值归零。
2. **正例只用 `Point`**：`simple()` 的 O0 汇编只有 23 行、O3 只有 2 行，
   对照极其干净（`v[0]` + `push` 的版本是 O0=794 行 / O3=251 行）。
