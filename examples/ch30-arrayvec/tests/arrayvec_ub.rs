#![cfg(miri)]

use std::mem::MaybeUninit;

struct BrokenPrefix {
    len: usize,
    data: [MaybeUninit<bool>; 2],
}

impl BrokenPrefix {
    fn push_with(&mut self, make: impl FnOnce() -> bool) {
        self.len += 1;
        self.data[self.len - 1].write(make());
    }
}

impl Drop for BrokenPrefix {
    fn drop(&mut self) {
        for index in 0..self.len {
            std::hint::black_box(unsafe { self.data[index].assume_init_read() });
        }
    }
}

#[test]
fn publishing_length_before_construction_reads_an_uninitialized_slot_on_unwind() {
    let mut broken = BrokenPrefix {
        len: 0,
        data: [MaybeUninit::uninit(); 2],
    };
    broken.push_with(|| true);
    broken.push_with(|| panic!("constructor failed"));
}
