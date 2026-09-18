// 运行期演示：4 个 tokio 任务并发计数
//
// 复现：cargo run -p ch22-tokio
// 预期输出：total = 4

use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() {
    let counter = Arc::new(Mutex::new(0u64));
    let mut handles = Vec::new();

    for _ in 0..4 {
        let c = Arc::clone(&counter);
        handles.push(tokio::spawn(async move {
            // ★ 块作用域：guard 必须在 await 之前 drop（第 20 章）
            let cur = { let g = c.lock().unwrap(); *g };
            tokio::task::yield_now().await;
            let mut g = c.lock().unwrap();
            *g = cur + 1;
        }));
    }

    for h in handles {
        h.await.expect("task panicked");
    }

    println!("total = {}", *counter.lock().unwrap());
}
