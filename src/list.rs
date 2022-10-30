//! List implementations

#![allow(unsafe_code)]

use alloc::{boxed::Box, vec::Vec};
use core::mem::MaybeUninit;

/// Dynamic list (grow or fixed)
pub(crate) enum DynList<T, const CAP: usize> {
    Grow(GrowList<T>),
    Fixed(FixedList<T, CAP>),
}

impl<T, const CAP: usize> DynList<T, CAP> {
    pub(crate) fn merge<const PAC: usize>(
        self: &mut Box<Self>,
        mut other: Box<DynList<T, PAC>>,
    ) {
        use DynList::*;
        match **self {
            Grow(ref mut grow) => {
                match *other {
                    Grow(ref mut l) => grow.0.extend(l.0.drain(..)),
                    Fixed(ref mut l) => {
                        grow.0.extend(
                            l.data[..l.size]
                                .iter()
                                .map(|x| unsafe { x.assume_init_read() }),
                        );
                        // Don't drop items from other list
                        core::mem::forget(other);
                    }
                }
            }
            Fixed(ref mut list) => {
                match *other {
                    Grow(mut l) => {
                        l.0.splice(
                            ..0,
                            list.data[..list.size]
                                .iter()
                                .map(|x| unsafe { x.assume_init_read() }),
                        );
                        let mut new = Grow(l);
                        core::mem::swap(&mut **self, &mut new);
                        // Don't drop items from this list
                        core::mem::forget(new);
                    }
                    Fixed(ref mut l) => {
                        if l.len() + list.len() > CAP {
                            let mut vec = Vec::new();
                            vec.extend(
                                list.data[..list.size]
                                    .iter()
                                    .map(|x| unsafe { x.assume_init_read() }),
                            );
                            vec.extend(
                                l.data[..l.size]
                                    .iter()
                                    .map(|x| unsafe { x.assume_init_read() }),
                            );
                            let mut new = Grow(GrowList(vec));
                            core::mem::swap(&mut **self, &mut new);
                            // Don't drop items from this list
                            core::mem::forget(new);
                        } else {
                            for item in l.data[..l.size].iter() {
                                self.push(unsafe { item.assume_init_read() });
                            }
                        }
                        // Don't drop items from other list
                        core::mem::forget(other);
                    }
                }
            }
        }
    }
}

impl<T, const CAP: usize> Default for DynList<T, CAP> {
    fn default() -> Self {
        Self::Fixed(FixedList::default())
    }
}

pub(crate) struct GrowList<T>(Vec<T>);

impl<T> Default for GrowList<T> {
    fn default() -> Self {
        Self(Vec::default())
    }
}

pub(crate) struct FixedList<T, const CAP: usize> {
    size: usize,
    data: [MaybeUninit<T>; CAP],
}

impl<T, const CAP: usize> Default for FixedList<T, CAP> {
    fn default() -> Self {
        let size = 0;
        let data = uninit_array::<T, CAP>();

        Self { size, data }
    }
}

pub(crate) trait List<T> {
    fn push(&mut self, item: T);
    fn pop(&mut self) -> Option<T>;
    fn len(&self) -> usize;
    // fn get(&mut self, index: usize) -> &mut T;
}

impl<T, const CAP: usize> List<T> for DynList<T, CAP> {
    fn push(&mut self, item: T) {
        match self {
            DynList::Grow(ref mut list) => list.push(item),
            DynList::Fixed(ref mut list) => {
                if list.len() == CAP {
                    let mut vec =
                        Vec::from(unsafe { array_assume_init(&list.data) });
                    vec.push(item);
                    *self = DynList::Grow(GrowList(vec));
                } else {
                    list.push(item);
                }
            }
        }
    }

    fn pop(&mut self) -> Option<T> {
        match self {
            DynList::Grow(ref mut list) => list.pop(),
            DynList::Fixed(ref mut list) => list.pop(),
        }
    }

    fn len(&self) -> usize {
        match self {
            DynList::Grow(ref list) => list.len(),
            DynList::Fixed(ref list) => list.len(),
        }
    }

    /*fn get(&mut self, index: usize) -> &mut T {
        match self {
            DynList::Grow(ref mut list) => list.get(index),
            DynList::Fixed(ref mut list) => list.get(index),
        }
    }*/
}

impl<T> List<T> for GrowList<T> {
    fn push(&mut self, item: T) {
        self.0.push(item);
    }

    fn pop(&mut self) -> Option<T> {
        self.0.pop()
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    /*fn get(&mut self, index: usize) -> &mut T {
        self.0.get_mut(index).unwrap()
    }*/
}

impl<T, const CAP: usize> List<T> for FixedList<T, CAP> {
    fn push(&mut self, item: T) {
        assert_ne!(self.size, CAP);
        self.data[self.size].write(item);
        self.size += 1;
    }

    fn pop(&mut self) -> Option<T> {
        if self.size == 0 {
            None
        } else {
            self.size -= 1;
            Some(unsafe { self.data[self.size].assume_init_read() })
        }
    }

    fn len(&self) -> usize {
        self.size
    }

    /*fn get(&mut self, index: usize) -> &mut T {
        assert!(index < self.size);

        unsafe { self.data[index].assume_init_mut() }
    }*/
}

impl<T, const CAP: usize> Drop for FixedList<T, CAP> {
    fn drop(&mut self) {
        for item in self.data[..self.size].iter_mut() {
            unsafe { item.assume_init_drop() }
        }
    }
}

/// Can be removed once https://github.com/rust-lang/rust/issues/96097 resolves
#[must_use]
#[inline(always)]
const fn uninit_array<T, const N: usize>() -> [MaybeUninit<T>; N] {
    // SAFETY: An uninitialized `[MaybeUninit<_>; LEN]` is valid.
    unsafe { MaybeUninit::<[MaybeUninit<T>; N]>::uninit().assume_init() }
}

#[inline(always)]
unsafe fn array_assume_init<T, const N: usize>(
    array: &[MaybeUninit<T>; N],
) -> [T; N] {
    // SAFETY:
    // * The caller guarantees that all elements of the array are initialized
    // * `MaybeUninit<T>` and T are guaranteed to have the same layout
    // * `MaybeUninit` does not drop, so there are no double-frees
    // And thus the conversion is safe
    let array: *const _ = array;
    let array: *const [T; N] = array.cast();

    array.read()
}
