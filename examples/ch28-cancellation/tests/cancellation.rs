use ch28_cancellation::{
    abort_runs_drop, queue_after_safe_cancel, queue_after_unsafe_cancel, select_drops_loser,
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
