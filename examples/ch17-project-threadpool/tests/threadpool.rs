use ch17_project_threadpool::{ThreadPool, pool_sum, spawn_per_task};

#[test]
fn pool_sum_handles_empty_and_multiple_chunks() {
    assert_eq!(pool_sum(1, Vec::new()), 0);
    assert_eq!(pool_sum(2, vec![1, 2, 3, 4, 5, 6, 7]), 28);
}

#[test]
fn per_task_baseline_has_the_same_result() {
    let data = vec![10, 20, 30, 40, 50];
    assert_eq!(spawn_per_task(data), 150);
}

#[test]
#[should_panic(expected = "线程池至少需要一个 worker")]
fn zero_workers_is_rejected() {
    let _ = ThreadPool::new(0);
}
