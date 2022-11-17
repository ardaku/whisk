use core::{
    cell::UnsafeCell,
    sync::atomic::{
        AtomicBool,
        Ordering::{Acquire, Release},
    },
};

/// Mutex
pub(crate) struct Mutex<T> {
    lock: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Create a new mutex
    pub(crate) const fn new(value: T) -> Self {
        let lock = AtomicBool::new(false);
        let data = UnsafeCell::new(value);

        Self { lock, data }
    }

    /// Try to lock the mutex
    pub(crate) fn try_with<R>(
        &self,
        then: impl FnOnce(Option<&mut T>) -> R,
    ) -> R {
        if self.lock.swap(true, Acquire) {
            then(None)
        } else {
            let ret = then(Some(unsafe { &mut *self.data.get() }));

            self.lock.store(false, Release);

            ret
        }
    }
}
