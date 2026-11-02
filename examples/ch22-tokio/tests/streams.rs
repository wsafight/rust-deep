use std::future::ready;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

#[tokio::test(flavor = "current_thread")]
async fn a_bounded_stream_applies_backpressure_until_consumed() {
    let (sender, receiver) = mpsc::channel(1);
    sender.send(1u64).await.unwrap();
    let mut stream = ReceiverStream::new(receiver);
    {
        let sending = sender.send(2);
        tokio::pin!(sending);
        tokio::select! {
            biased;
            _ = &mut sending => panic!("a full channel must wait"),
            _ = ready(()) => {}
        }
        assert_eq!(stream.next().await, Some(1));
        sending.await.unwrap();
    }
    drop(sender);
    assert_eq!(stream.next().await, Some(2));
    assert_eq!(stream.next().await, None);
}

#[tokio::test(flavor = "current_thread")]
async fn consuming_a_stream_does_not_require_spawning_each_item() {
    let stream = tokio_stream::iter([1u64, 2, 3]);
    let total: u64 = stream.fold(0, |sum, value| sum + value).await;
    assert_eq!(total, 6);
}
