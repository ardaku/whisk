use core::{
    cell::{Cell, UnsafeCell},
    sync::atomic::{
        AtomicBool,
        Ordering::{Acquire, Release},
    },
    task::{Context, Poll},
};

use crate::wake_list::{WakeHandle, WakeList};

/// Mutex
pub(crate) struct Mutex<T> {
    /// True if mutex is currently being accessed
    lock: AtomicBool,
    /// Data in transit
    data: UnsafeCell<Option<T>>,
    /// List of waiting senders
    send: WakeList,
    /// List of waiting receivers
    recv: WakeList,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    /// Create a new mutex
    pub(crate) const fn new() -> Self {
        let lock = AtomicBool::new(false);
        let data = UnsafeCell::new(None);
        let send = WakeList::new();
        let recv = WakeList::new();

        Self {
            lock,
            data,
            send,
            recv,
        }
    }

    /// Try to store data in the mutex
    pub(crate) fn store(
        &self,
        data: &Cell<Option<T>>,
        cx: &mut Context<'_>,
        wh: &mut WakeHandle,
    ) -> Poll<()> {
        // Try to acquire lock
        if self.lock.swap(true, Acquire) {
            // Data is contended, register sender to wake list
            wh.register(&self.send, cx.waker().clone());

            // Try again just in case registration is unnecessary
            if self.lock.swap(true, Acquire) {
                // Will be awoken
                return Poll::Pending;
            } else {
                // Locked, and registered
                // If can't send until receive
                if unsafe { (*self.data.get()).is_some() } {
                    // Release lock
                    self.lock.store(false, Release);
                    // Wake a receiver
                    self.recv.wake_one();
                    return Poll::Pending;
                }

                // Registration was unnecessary, unregister
                *wh = WakeHandle::new();
            }
        } else {
            // Locked, but not registered
            // If can't send until receive, register and return
            if unsafe { (*self.data.get()).is_some() } {
                // Register
                wh.register(&self.send, cx.waker().clone());
                // Release lock
                self.lock.store(false, Release);
                // Wake a receiver
                self.recv.wake_one();
                return Poll::Pending;
            }
        }

        // Write to inner data
        unsafe { (*self.data.get()) = data.take() };

        // Release lock
        self.lock.store(false, Release);

        // Wake a receiver
        self.recv.wake_one();

        Poll::Ready(())
    }

    /// Try to take data from the mutex
    pub(crate) fn take(
        &self,
        cx: &mut Context<'_>,
        wh: &mut WakeHandle,
    ) -> Poll<T> {
        // Try to acquire lock
        if self.lock.swap(true, Acquire) {
            // Data is contended, register sender to wake list
            wh.register(&self.recv, cx.waker().clone());

            // Try again just in case registration is unnecessary
            if self.lock.swap(true, Acquire) {
                // Will be awoken
                return Poll::Pending;
            } else {
                // Locked, and registered
                // If can't receive until send
                if unsafe { (*self.data.get()).is_none() } {
                    // Release lock
                    self.lock.store(false, Release);
                    // Wake a sender
                    self.send.wake_one();
                    return Poll::Pending;
                }

                // Registration was unnecessary, unregister
                *wh = WakeHandle::new();
            }
        } else {
            // If can't receive until send, register and return
            if unsafe { (*self.data.get()).is_none() } {
                // Register
                wh.register(&self.recv, cx.waker().clone());
                // Release lock
                self.lock.store(false, Release);
                // Wake a sender
                self.send.wake_one();
                return Poll::Pending;
            }
        }

        // Take from inner data
        let ret = if let Some(data) = unsafe { (*self.data.get()).take() } {
            Poll::Ready(data)
        } else {
            Poll::Pending
        };

        // Release lock
        self.lock.store(false, Release);

        // Wake a sender
        self.send.wake_one();

        // Return `Ready(_)`
        ret
    }
}
