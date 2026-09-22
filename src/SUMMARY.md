# Summary

[前言](./preface.md)

---

# 第一部分：所有权的心智模型

- [1. 借用检查器到底在检查什么](./part1/01-borrow-checker.md)
- [2. 生命周期：标注、省略与推断](./part1/02-lifetimes.md)
- [3. 协变、逆变与不变](./part1/03-variance.md)
- [4. 自引用结构与 Pin 的前置知识](./part1/04-self-referential.md)
- [5. 实战：写一个安全的树 / 图容器](./part1/05-project-tree.md)

---

# 第二部分：Trait 系统深水区

- [6. 关联类型 vs 泛型参数](./part2/06-associated-types.md)
- [7. dyn vs 泛型：dyn compatibility 与单态化代价](./part2/07-dyn-vs-generics.md)
- [8. coherence、孤儿规则与 blanket impl](./part2/08-coherence.md)
- [9. 高阶 trait bound（HRTB）](./part2/09-hrtb.md)
- [10. GAT：泛型关联类型](./part2/10-gat.md)
- [11. 实战：设计一个小型 trait 抽象层](./part2/11-project-abstract.md)

---

# 第三部分：并发

- [12. Send 与 Sync 的真相](./part3/12-send-sync.md)
- [13. 线程、Arc 与 Mutex](./part3/13-threads-arc-mutex.md)
- [14. 消息传递：channel 与所有权转移](./part3/14-channels.md)
- [15. 共享可变状态的所有权设计](./part3/15-shared-state-design.md)
- [16. 无锁与 atomics 入门](./part3/16-atomics.md)
- [17. 实战：构造一个并发任务池](./part3/17-project-threadpool.md)

---

# 第四部分：异步

- [18. Future 是惰性的：手写一个最小 executor](./part4/18-future-is-lazy.md)
- [19. Pin 与 Unpin 为什么存在](./part4/19-pin-unpin.md)
- [20. async 中的生命周期与 Send 传染](./part4/20-async-lifetimes.md)
- [21. async fn in trait 的现状](./part4/21-afit.md)
- [22. tokio 实战：从原理到工程](./part4/22-tokio.md)
- [23. 实战：写一个 mini async runtime](./part4/23-project-runtime.md)

---

# 第五部分：unsafe 与 soundness

- [24. unsafe 的边界哲学：用 unsafe 实现安全接口](./part5/24-unsafe-boundary.md)
- [25. 别名规则、UnsafeCell 与 PhantomData](./part5/25-aliasing.md)
- [26. 何时不该用 unsafe](./part5/26-when-not-to.md)

---

# 附录

- [A. 环境与版本（Rust 1.98.1 / edition 2024）](./appendix/a-setup.md)
- [B. 常见编译错误逐条解读](./appendix/b-errors.md)
- [C. 延伸阅读](./appendix/c-reading.md)
