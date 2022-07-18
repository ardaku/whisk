//! Simple implementation of a spinlock

use core::{
    cell::UnsafeCell,
    sync::atomic::{
        self, AtomicBool,
        Ordering::{Acquire, Relaxed, Release},
    },
};

/// A simple spinlock mutex
#[derive(Debug)]
pub(crate) struct SpinLock<T: Send + Sync>(UnsafeCell<Option<T>>, AtomicBool);

unsafe impl<T: Send + Sync> Sync for SpinLock<T> {}

impl<T: Send + Sync> Default for SpinLock<T> {
    fn default() -> Self {
        Self(None.into(), false.into())
    }
}

impl<T: Send + Sync> SpinLock<T> {
    pub(crate) fn store(&self, value: Option<T>) {
        while self
            .1
            .compare_exchange_weak(false, true, Relaxed, Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        atomic::fence(Acquire);
        unsafe { *self.0.get() = value };
        self.1.store(false, Release);
    }

    pub(crate) fn take(&self) -> Option<T> {
        while self
            .1
            .compare_exchange_weak(false, true, Relaxed, Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        atomic::fence(Acquire);
        let value = unsafe { (*self.0.get()).take() };
        self.1.store(false, Release);
        value
    }
}
