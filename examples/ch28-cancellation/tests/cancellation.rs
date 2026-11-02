use ch28_cancellation::{
    abort_runs_drop, preserved_read_keeps_prefix, queue_after_safe_cancel,
    queue_after_unsafe_cancel, recreated_read_loses_prefix, select_drops_loser,
    timeout_drops_inner,
};

#[tokio::test(flavor = "current_thread")]
async fn abort_drops_the_future_state() {
    assert_eq!(abort_runs_drop().await, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn select_drops_the_losing_branch() {
    assert_eq!(select_drops_loser().await, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_drops_the_inner_future() {
    assert_eq!(timeout_drops_inner().await, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_does_not_rollback_side_effects() {
    assert!(queue_after_unsafe_cancel().await.is_empty());
    assert_eq!(queue_after_safe_cancel().await, vec![7]);
}

#[tokio::test(flavor = "current_thread")]
async fn recreating_a_read_in_select_discards_consumed_bytes() {
    assert_eq!(recreated_read_loses_prefix().await, [3, 4]);
}

#[tokio::test(flavor = "current_thread")]
async fn keeping_the_same_future_preserves_its_partial_state() {
    assert_eq!(preserved_read_keeps_prefix().await, [1, 2]);
}

#[tokio::test(flavor = "current_thread")]
async fn graceful_shutdown_waits_for_the_task_to_release_resources() {
    assert_eq!(ch28_cancellation::shutdown_and_join().await, 1);
}
