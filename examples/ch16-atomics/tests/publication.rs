use loom::sync::Arc;
use loom::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use loom::thread;

fn publication(store_order: Ordering, load_order: Ordering) {
    loom::model(move || {
        let data = Arc::new(AtomicUsize::new(0));
        let ready = Arc::new(AtomicBool::new(false));
        let writer_data = Arc::clone(&data);
        let writer_ready = Arc::clone(&ready);
        let writer = thread::spawn(move || {
            writer_data.store(42, Ordering::Relaxed);
            writer_ready.store(true, store_order);
        });
        let reader = thread::spawn(move || {
            if ready.load(load_order) {
                assert_eq!(data.load(Ordering::Relaxed), 42, "published data");
            }
        });
        writer.join().unwrap();
        reader.join().unwrap();
    });
}

#[test]
fn release_acquire_publishes_the_data() {
    publication(Ordering::Release, Ordering::Acquire);
}

#[test]
#[should_panic(expected = "published data")]
fn relaxed_flag_does_not_publish_the_data() {
    publication(Ordering::Relaxed, Ordering::Relaxed);
}
