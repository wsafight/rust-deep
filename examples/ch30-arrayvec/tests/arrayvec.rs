use ch30_arrayvec::ArrayVec;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct Counted {
    id: usize,
    drops: Arc<AtomicUsize>,
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn counted(id: usize, drops: &Arc<AtomicUsize>) -> Counted {
    Counted {
        id,
        drops: Arc::clone(drops),
    }
}

#[test]
fn push_pop_insert_and_remove_preserve_order() {
    let mut vec = ArrayVec::<u64, 4>::new();
    assert_eq!(vec.try_push(1), Ok(()));
    assert_eq!(vec.try_push(3), Ok(()));
    assert_eq!(vec.try_insert(1, 2), Ok(()));
    assert_eq!(&*vec, &[1, 2, 3]);
    assert_eq!(vec.remove(1), 2);
    assert_eq!(vec.pop(), Some(3));
    assert_eq!(&*vec, &[1]);
}

#[test]
fn full_vector_returns_ownership() {
    let mut vec = ArrayVec::<String, 1>::new();
    vec.try_push("first".into()).unwrap();
    let second = vec.try_push("second".into()).unwrap_err();
    assert_eq!(second, "second");
    assert_eq!(&*vec, &[String::from("first")]);
}

#[test]
fn every_value_is_dropped_exactly_once() {
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let mut vec = ArrayVec::<Counted, 4>::new();
        vec.try_push(counted(1, &drops)).unwrap();
        vec.try_push(counted(2, &drops)).unwrap();
        drop(vec.pop());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn partial_construction_cleans_the_prefix() {
    let drops = Arc::new(AtomicUsize::new(0));
    let result = ArrayVec::<Counted, 4>::try_from_fn(|index| {
        if index == 2 {
            Err("stop")
        } else {
            Ok(counted(index, &drops))
        }
    });
    match result {
        Err(error) => assert_eq!(error, "stop"),
        Ok(_) => panic!("construction should stop at index 2"),
    }
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn panicking_construction_also_cleans_the_prefix() {
    let drops = Arc::new(AtomicUsize::new(0));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = ArrayVec::<Counted, 4>::try_from_fn::<()>(|index| {
            if index == 2 {
                panic!("construction failed");
            }
            Ok(counted(index, &drops))
        });
    }));
    assert!(result.is_err());
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn dropping_into_iter_cleans_unconsumed_values() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut vec = ArrayVec::<Counted, 4>::new();
    for id in 0..3 {
        vec.try_push(counted(id, &drops)).unwrap();
    }

    let mut iter = vec.into_iter();
    let first = iter.next().unwrap();
    assert_eq!(first.id, 0);
    drop(first);
    drop(iter);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
}

#[test]
fn zero_capacity_and_zero_sized_types_work() {
    let mut empty = ArrayVec::<u64, 0>::new();
    assert_eq!(empty.try_push(1), Err(1));

    let mut units = ArrayVec::<(), 3>::new();
    assert_eq!(units.try_push(()), Ok(()));
    assert_eq!(units.try_push(()), Ok(()));
    assert_eq!(units.len(), 2);
    assert_eq!(units.into_iter().count(), 2);
}
