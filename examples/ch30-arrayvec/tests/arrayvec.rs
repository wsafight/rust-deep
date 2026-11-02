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

#[test]
fn retain_preserves_order_and_mutations() {
    let mut values = ArrayVec::<usize, 5>::new();
    values.try_extend(0..5).unwrap();
    values.retain_mut(|value| {
        *value += 10;
        *value % 2 == 0
    });
    assert_eq!(values.as_slice(), &[10, 12, 14]);
    values.retain_mut(|_| false);
    assert!(values.is_empty());
}

#[test]
fn retain_repairs_the_prefix_when_the_callback_panics() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut values = ArrayVec::<Counted, 4>::new();
    for id in 0..4 {
        values.try_push(counted(id, &drops)).unwrap();
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        values.retain_mut(|value| match value.id {
            0 => false,
            2 => panic!("predicate failed"),
            _ => true,
        });
    }));
    assert!(result.is_err());
    assert_eq!(
        values.iter().map(|value| value.id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(values);
    assert_eq!(drops.load(Ordering::SeqCst), 4);
}

#[test]
fn retain_does_not_redrop_an_element_whose_destructor_panics() {
    struct PanickingDrop {
        id: usize,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for PanickingDrop {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            assert_ne!(self.id, 1, "destructor failed");
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let mut values = ArrayVec::<PanickingDrop, 3>::new();
    for id in 0..3 {
        assert!(
            values
                .try_push(PanickingDrop {
                    id,
                    drops: Arc::clone(&drops)
                })
                .is_ok()
        );
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        values.retain_mut(|value| value.id == 2);
    }));
    assert!(result.is_err());
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].id, 2);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    drop(values);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
}

#[test]
fn cloning_a_panicking_element_cleans_only_completed_clones() {
    #[derive(Debug)]
    struct FallibleClone(Counted);

    impl Clone for FallibleClone {
        fn clone(&self) -> Self {
            assert_ne!(self.0.id, 2, "clone failed");
            Self(counted(self.0.id, &self.0.drops))
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let mut values = ArrayVec::<FallibleClone, 4>::new();
    for id in 0..4 {
        values.try_push(FallibleClone(counted(id, &drops))).unwrap();
    }
    let result = std::panic::catch_unwind(|| values.clone());
    assert!(result.is_err());
    assert_eq!(values.len(), 4);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    drop(values);
    assert_eq!(drops.load(Ordering::SeqCst), 6);
}

#[test]
fn extend_keeps_completed_items_if_the_iterator_panics() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut values = ArrayVec::<Counted, 4>::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let items = (0..4).map(|id| {
            assert_ne!(id, 2, "iterator failed");
            counted(id, &drops)
        });
        assert!(values.try_extend(items).is_ok());
    }));
    assert!(result.is_err());
    assert_eq!(values.len(), 2);
    drop(values);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn extend_returns_the_first_item_that_does_not_fit() {
    let mut values = ArrayVec::<usize, 2>::new();
    assert_eq!(values.try_extend(0..4), Err(2));
    assert_eq!(values.as_slice(), &[0, 1]);
}

#[test]
fn zero_sized_values_with_drop_are_counted_logically() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct ZeroSized;

    impl Drop for ZeroSized {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    assert_eq!(std::mem::size_of::<ZeroSized>(), 0);
    let mut values = ArrayVec::<ZeroSized, 4>::new();
    for _ in 0..4 {
        assert!(values.try_push(ZeroSized).is_ok());
    }
    let mut seen = 0;
    values.retain_mut(|_| {
        seen += 1;
        seen % 2 == 0
    });
    assert_eq!(values.len(), 2);
    drop(values.into_iter().next());
    assert_eq!(DROPS.load(Ordering::SeqCst), 4);
}

#[test]
fn forgetting_an_iterator_does_not_drop_or_reuse_its_remaining_items() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct ZeroSized;

    impl Drop for ZeroSized {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    let mut values = ArrayVec::<ZeroSized, 3>::new();
    for _ in 0..3 {
        assert!(values.try_push(ZeroSized).is_ok());
    }
    let mut iterator = values.into_iter();
    drop(iterator.next());
    std::mem::forget(iterator);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}
