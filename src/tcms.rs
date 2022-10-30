//! Lockless synchronization

#![allow(unsafe_code)]

use alloc::boxed::Box;
use core::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

/// Take-create-merge-swap
///
/// Essentially the opposite of RCU (read-copy-update), optimized for writing
/// rather than reading.
///
///  - Task 1 takes the inner value
///  - Task 2 sees Task 1 has ownership of value
///  - Task 2 creates a new empty/default value
///  - Task 2 writes to new value
///  - Task 1 returns value
///  - Task 2 checks if value has been returned and swaps if not
///  - Task 2 takes ownership of other value if returned and merges then returns
///
/// One thing to keep in mind when using this type is that not all values will
/// be available at all times.
pub(crate) struct Tcms<T: Default>(AtomicPtr<T>);

impl<T: Default> Tcms<T> {
    /// Create new TCMS
    pub(crate) fn new() -> Self {
        Self(AtomicPtr::new(Box::into_raw(Box::new(T::default()))))
    }

    /// Run `f` with merger `m`.
    ///
    /// Merger is unstable, can't expect order to be preserved
    pub(crate) fn with<R>(
        &self,
        f: impl FnOnce(&mut T) -> R,
        m: impl Fn(&mut Box<T>, Box<T>),
    ) -> R {
        // Swap with null pointer
        let list = self.0.swap(ptr::null_mut(), Ordering::Acquire);
        let mut list = if list.is_null() {
            Box::new(T::default())
        } else {
            unsafe { Box::from_raw(list) }
        };

        // Run closure with list
        let r = f(&mut *list);

        // Merge lists if needed
        let mut new = Box::into_raw(list);
        while self
            .0
            .compare_exchange(
                core::ptr::null_mut(),
                new,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_err()
        {
            let other = self.0.swap(ptr::null_mut(), Ordering::Acquire);
            if !other.is_null() {
                let mut a = unsafe { Box::from_raw(new) };
                let b = unsafe { Box::from_raw(other) };
                m(&mut a, b);
                new = Box::into_raw(a);
            } else {
                // Too much contention with other task, try again
                core::hint::spin_loop();
            };
        }

        r
    }
}
