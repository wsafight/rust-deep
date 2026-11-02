# 评审驱动改造记录

日期：2026-11-03。基线：`b39b62e`。顺序：先纠正事实与验证语义，再补真实缺口，
最后改善阅读路径；不以篇幅或反例数量单独判断一章是否完成。

## P0：正确性与核心心智模型

- 第 1 章：增加条件返回借用的 NLL 分析边界与安全改写；修正“只在一个分支使用
  就报错”“删掉读取会延长借用”“通配模式等于读取引用”三处错误说明。
- 第 2 章：补 RPIT 默认捕获、`use<>` / `use<'a>`、outlives 与捕获集合的区别，
  正例和 E0700 等反例同时进入验证。
  澄清 longest_elided 的历史命名：其签名仍显式标注，alias 不是省略规则的证据。
- 第 12 章：将捕获方式、调用 trait、Send 拆开讲；验证 Fn / FnMut / FnOnce，
  明确 move 不意味着只能调用一次，也不保证 `'static`。
- 第 16 章：保留有价值的汇编对照，增加 Loom 发布协议正反模型。只将 Loom
  放入 dev-dependencies，不改变代码生成样本的依赖。
- 第 28 章：增加 select 循环重建读取与保存同一 future 的确定性对照，补协作
  停机与 await JoinHandle，以及 Drop 不能 await 的编译期反例。
- 第 30 章：补 Clone、try_extend、retain_mut 及恢复前缀的 RetainGuard；覆盖用户
  predicate、元素 Clone、iterator 与元素析构 panic；加入预期 UB 的 Miri 探针。
- 附录 D.0：让已有的工具边界更显眼，明确区分动态路径检测与有界模型探索。

## P1：读者入口与工程主题

- 新增第 0 章：MIR 基本块、局部槽位、copy / move、cleanup、LLVM 属性与常见
  双架构指令速查；提供练习和在线工具入口，不假定网站配置等于本书基线。
- 增加四张可访问 SVG：借用控制流、Pin 两层结构、Future 状态与 trait object。
  布局图明确是示意，不暗示稳定 ABI。
- 第 7 章补 trait upcasting 及逆向转换反例；不把数据地址不变等同于 vtable 不变。
- 第 11 章补错误类型、From / `?`、source 与 `Box<dyn Error + Send + Sync>`。
- 第 22 章补 Stream 与有界队列背压，测试 Pending、恢复与结束语义。
- 第 29 章增加可实际编译运行的 C 调用方、头文件与 native staticlib 集成探针。
- mdBook 显式采用 Edition 2024；跨平台脚本为子进程固定当前工具链，避免书稿
  测试与工作区测试使用不同的 edition 或默认 rustdoc。

## 阅读体验与 CI

- 15 段较长的 MIR / LLVM IR / 汇编证据使用原生 details 默认折叠。
- 第 19、22、25 章提供分层阅读卡片；保留现有章节号和深链接，不立即拆章重排。
- README 先说明读者定位、路线与样章，再放工程复现命令；移除“一次完成”口径。
- 普通测试与 C ABI 探针通过 Ubuntu / macOS CI 矩阵运行；固定 macOS 汇编证据
  与 Linux Miri 门禁继续分开，不把交叉编译写成运行验证。

## 未机械采纳的建议

- 附录 D 原本就有证据边界；第 30 章原本就有 IntoIter、零容量、ZST 和部分
  初始化测试；第 28 章也并非只有文字，因此保留并扩展，不推倒重写。
- 不为使用 `#[may_dangle]` 而引入不稳定机制；内联 MaybeUninit 数组已经表达拥有 T。
- 不按“每章必须 15 处强调”统一删改技术内容，不为提示框引入额外 mdBook 插件。
- 大规模拆章、全书练习题体系与逐段文风精修留给独立编辑轮次，避免在正确性
  改造中破坏章节引用或未经核对地压缩论证。

## 复现

```bash
scripts/verify-portable.sh
scripts/verify-evidence.sh
scripts/verify-miri.sh
```

本轮示例目录仍为 31 个；compile-fail 文件从 55 个增加到 63 个。Miri 为
10 个目标加一项前置检查，预期 UB 目标报告 UB 才计为通过。以上不是 Linux
远程 CI 的运行结果声明，实际远程状态应以 Actions 记录为准。
