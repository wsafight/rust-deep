// 第 8 章 · 两 crate 探针 —— 上游（"别人的 crate"）
//
// 复现：examples/ch08-coherence/cross-crate/run.sh
// 这里定义一个**外部的** trait，供 downstream.rs 尝试 blanket impl。

pub trait Format {
    fn fmt_it(&self) -> String;
}

pub struct Doc(pub String);

impl Format for Doc {
    fn fmt_it(&self) -> String { self.0.clone() }
}
