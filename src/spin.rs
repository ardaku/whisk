//! Simple implementation of a spinlock

use core::{
    cell::UnsafeCell,
    sync::atomic::{
        self, AtomicBool,
        Ordering::{Acquire, Relaxed, Release},
    },
};

/// A simple spinlock mutex
#[derive(Debug, Default)]
pub(crate) struct SpinLock<T: Send + Default>(UnsafeCell<T>, AtomicBool);

unsafe impl<T: Send + Default> Sync for SpinLock<T> {}

impl<T: Send + Default> SpinLock<T> {
    #[inline(always)]
    pub(crate) fn with<O>(&self, then: impl FnOnce(&mut T) -> O) -> O {
        while self
            .1
            .compare_exchange_weak(false, true, Relaxed, Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        atomic::fence(Acquire);
        let output = unsafe { then(&mut *self.0.get()) };
        self.1.store(false, Release);
        output
    }
}
