// 运行期演示：mini async runtime 跑三个任务
//
// 复现：cargo run -p ch23-project-runtime
// 预期输出：
//   order  = [0, 1, 2]
//   count  = 5
//   local  = 42

use ch23_project_runtime::{block_on, count_to, run_local, run_three};

fn main() {
    println!("order  = {:?}", run_three());
    println!("count  = {}", block_on(count_to(5)));
    println!("local  = {}", run_local());
}
