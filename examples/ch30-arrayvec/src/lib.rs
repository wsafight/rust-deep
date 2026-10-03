//! 第 30 章（实战）：实现一个安全的 `ArrayVec<T, N>`
//!
//! 证据生成：`tools/evidence.sh ch30-arrayvec`
//!
//! # Safety invariant
//!
//! 对每个 `ArrayVec<T, N>`：
//!
//! 1. `len <= N`；
//! 2. `data[..len]` 恰好包含 `len` 个已初始化、有效且由容器拥有的 `T`；
//! 3. `data[len..]` 不得作为 `T` 读取或析构。

use std::iter::FusedIterator;
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::{Deref, DerefMut};
use std::ptr;
use std::slice;

pub struct ArrayVec<T, const N: usize> {
    len: usize,
    data: [MaybeUninit<T>; N],
}

impl<T, const N: usize> ArrayVec<T, N> {
    pub const fn new() -> Self {
        Self {
            len: 0,
            data: [const { MaybeUninit::uninit() }; N],
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn is_full(&self) -> bool {
        self.len == N
    }

    pub fn as_slice(&self) -> &[T] {
        // SAFETY: invariant 2 says the first len slots are initialized T; the array provides
        // a non-null, aligned pointer, and invariant 1 proves len is in bounds.
        unsafe { slice::from_raw_parts(self.data.as_ptr().cast::<T>(), self.len) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: same initialized-prefix invariant as as_slice; &mut self guarantees unique
        // access to the elements for the returned borrow.
        unsafe { slice::from_raw_parts_mut(self.data.as_mut_ptr().cast::<T>(), self.len) }
    }

    /// Adds one value without allocation. On a full vector, ownership is returned to the caller.
    pub fn try_push(&mut self, value: T) -> Result<(), T> {
        if self.len == N {
            return Err(value);
        }

        // Write first, publish the new length second. If future implementations make the write
        // path more complex, unwind before len changes still leaves the old invariant intact.
        self.data[self.len].write(value);
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }

        // Remove the slot from the initialized prefix before transferring its value out.
        self.len -= 1;
        // SAFETY: old len was positive, so the slot at the new len was initialized. Decrementing
        // len first ensures ArrayVec::drop will not drop the moved-out value during unwinding.
        Some(unsafe { self.data.get_unchecked(self.len).assume_init_read() })
    }

    /// Inserts at `index`; panics when index is greater than len and returns the value when full.
    pub fn try_insert(&mut self, index: usize, value: T) -> Result<(), T> {
        assert!(index <= self.len, "index out of bounds");
        if self.len == N {
            return Err(value);
        }

        // SAFETY: index <= len < N. ptr::copy supports overlap and shifts the initialized suffix
        // one slot right. No operation between copy, write, and len update can unwind.
        unsafe {
            let base = self.data.as_mut_ptr().cast::<T>();
            ptr::copy(base.add(index), base.add(index + 1), self.len - index);
            base.add(index).write(value);
        }
        self.len += 1;
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> T {
        assert!(index < self.len, "index out of bounds");

        // SAFETY: index points into the initialized prefix. Read transfers that element out;
        // ptr::copy shifts the suffix over the hole. The duplicate bytes beyond the new len are
        // intentionally outside the initialized prefix and will never be read or dropped.
        unsafe {
            let base = self.data.as_mut_ptr().cast::<T>();
            let value = base.add(index).read();
            ptr::copy(base.add(index + 1), base.add(index), self.len - index - 1);
            self.len -= 1;
            value
        }
    }

    pub fn clear(&mut self) {
        // pop updates len before each element's destructor runs. If T::drop panics, the remaining
        // initialized prefix is still described by len and can be cleaned up later.
        while let Some(value) = self.pop() {
            drop(value);
        }
    }

    /// Builds elements left-to-right. Error return and panic both drop the initialized prefix.
    pub fn try_from_fn<E>(mut make: impl FnMut(usize) -> Result<T, E>) -> Result<Self, E> {
        let mut out = Self::new();
        while out.len < N {
            let value = make(out.len)?;
            match out.try_push(value) {
                Ok(()) => {}
                Err(_) => unreachable!("len < N was checked"),
            }
        }
        Ok(out)
    }
}

impl<T, const N: usize> Default for ArrayVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Deref for ArrayVec<T, N> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<T, const N: usize> DerefMut for ArrayVec<T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

impl<T, const N: usize> Drop for ArrayVec<T, N> {
    fn drop(&mut self) {
        // SAFETY: the invariant says exactly data[..len] is initialized. Dropping a raw slice
        // runs each remaining T destructor once and never touches the uninitialized suffix.
        unsafe {
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(
                self.data.as_mut_ptr().cast::<T>(),
                self.len,
            ));
        }
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a ArrayVec<T, N> {
    type Item = &'a T;
    type IntoIter = slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a mut ArrayVec<T, N> {
    type Item = &'a mut T;
    type IntoIter = slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

pub struct IntoIter<T, const N: usize> {
    data: [MaybeUninit<T>; N],
    next: usize,
    end: usize,
}

impl<T, const N: usize> Iterator for IntoIter<T, N> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }

        let index = self.next;
        self.next += 1;
        // SAFETY: [next, end) is the initialized, not-yet-yielded range. Incrementing next before
        // returning transfers this slot out of IntoIter's drop responsibility.
        Some(unsafe { self.data.get_unchecked(index).assume_init_read() })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.next;
        (remaining, Some(remaining))
    }
}

impl<T, const N: usize> ExactSizeIterator for IntoIter<T, N> {}
impl<T, const N: usize> FusedIterator for IntoIter<T, N> {}

impl<T, const N: usize> Drop for IntoIter<T, N> {
    fn drop(&mut self) {
        // SAFETY: [next, end) contains exactly the initialized elements not yielded yet.
        unsafe {
            let start = self.data.as_mut_ptr().cast::<T>().add(self.next);
            ptr::drop_in_place(ptr::slice_from_raw_parts_mut(start, self.end - self.next));
        }
    }
}

impl<T, const N: usize> IntoIterator for ArrayVec<T, N> {
    type Item = T;
    type IntoIter = IntoIter<T, N>;

    fn into_iter(self) -> Self::IntoIter {
        let this = ManuallyDrop::new(self);
        let len = this.len;
        // SAFETY: ManuallyDrop prevents ArrayVec::drop. Reading the MaybeUninit array transfers
        // responsibility for the initialized prefix to IntoIter; the source is never used again.
        let data = unsafe { ptr::read(&this.data) };
        IntoIter {
            data,
            next: 0,
            end: len,
        }
    }
}

/// Evidence wrapper: a four-element ArrayVec stays entirely on the stack.
#[unsafe(no_mangle)]
pub fn sum_arrayvec(values: [u64; 4]) -> u64 {
    let mut vec = ArrayVec::<u64, 4>::new();
    for value in values {
        let result = vec.try_push(value);
        debug_assert!(result.is_ok());
    }
    vec.iter().copied().sum()
}

#[unsafe(no_mangle)]
pub fn sum_array(values: [u64; 4]) -> u64 {
    values.into_iter().sum()
}
