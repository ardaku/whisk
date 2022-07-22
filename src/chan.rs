use alloc::{sync::Arc, vec::Vec};
use core::{
    cell::UnsafeCell,
    future::Future,
    mem::ManuallyDrop,
    pin::Pin,
    ptr,
    task::{
        Context,
        Poll::{self, Pending, Ready},
        Waker,
    },
    sync::atomic::{AtomicBool, Ordering::{Relaxed, Acquire, Release}, self},
};

// Copied from https://doc.rust-lang.org/core/sync/atomic/fn.fence.html#examples
#[derive(Debug, Default)]
struct Mutex {
    flag: AtomicBool,
}

impl Mutex {
    #[inline(always)]
    fn lock(&self) {
        // Wait until the old value is `false`.
        while self
            .flag
            .compare_exchange_weak(false, true, Relaxed, Relaxed)
            .is_err()
        {}
        // This fence synchronizes-with store in `unlock`.
        atomic::fence(Acquire);
    }

    #[inline(always)]
    fn unlock(&self) {
        self.flag.store(false, Release);
    }
}

#[derive(Debug)]
struct Locked<T: Send> {
    /// Address of data being sent
    addr: *mut ManuallyDrop<T>,
    /// Wakers
    wake: Vec<Waker>,
}

impl<T: Send> Default for Locked<T> {
    #[inline]
    fn default() -> Self {
        let addr = ptr::null_mut();
        let wake = Vec::new();

        Self { addr, wake }
    }
}

#[derive(Debug)]
struct Shared<T: Send> {
    data: UnsafeCell<Locked<T>>,
    mutex: Mutex,
}

impl<T: Send> Default for Shared<T> {
    #[inline]
    fn default() -> Self {
        let data = UnsafeCell::new(Locked::default());
        let mutex = Mutex::default();

        Shared { data, mutex }
    }
}

unsafe impl<T: Send> Send for Shared<T> {}
unsafe impl<T: Send> Send for Channel<T> {}

/// A `Channel` notifies when another `Channel` sends a message.
///
/// Implemented as a rendezvous multi-producer/multi-consumer queue
#[derive(Debug)]
pub struct Channel<T: Send>(Arc<Shared<T>>);

impl<T: Send> Clone for Channel<T> {
    #[inline]
    fn clone(&self) -> Self {
        Channel(Arc::clone(&self.0))
    }
}

impl<T: Send> Default for Channel<T> {
    #[inline]
    fn default() -> Self {
        Self(Arc::new(Shared::default()))
    }
}

impl<T: Send> Channel<T> {
    /// Create a new channel.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Send a message on this channel.
    #[inline(always)]
    pub fn send(&self, message: T) -> Message<T> {
        Message(
            (*self).clone(),
            ManuallyDrop::new(message),
            false,
        )
    }

    /// Receive a message from this channel.
    #[inline(always)]
    pub async fn recv(&self) -> T {
        self.clone().await
    }
}

impl<T: Send> Future for Channel<T> {
    type Output = T;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self;
        let waker = cx.waker();
        this.0.mutex.lock();
        let output = {
            let shared = unsafe { &mut *this.0.data.get() };
            let ptr = shared.addr;
            if !ptr.is_null() {
                let output = unsafe { ManuallyDrop::take(&mut ptr::read(ptr)) };
                shared.addr = ptr::null_mut();
                for waker in shared.wake.drain(..) {
                    waker.wake();
                }
                Ready(output)
            } else {
                shared.wake.push(waker.clone());
                // If a sender hasn't sent, return not ready
                Pending
            }
        };
        this.0.mutex.unlock();
        output
    }
}

/// A message in the process of being sent over a [`Channel`].
#[derive(Debug)]
pub struct Message<T: Send>(Channel<T>, ManuallyDrop<T>, bool);

impl<T: Send> Message<T> {
    #[inline(always)]
    fn pin_get_data(self: Pin<&mut Self>) -> Pin<&mut ManuallyDrop<T>> {
        // This is okay because `1` is pinned when `self` is.
        unsafe { self.map_unchecked_mut(|s| &mut s.1) }
    }

    #[inline(always)]
    fn pin_get_init(self: Pin<&mut Self>) -> &mut bool {
        // This is okay because `2` is never considered pinned.
        unsafe { &mut self.get_unchecked_mut().2 }
    }
}

impl<T: Send> Future for Message<T> {
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self;
        let waker = cx.waker();
        this.0.0.mutex.lock();
        let output = {
            let shared = unsafe { &mut *this.0.0.data.get() };
            shared.wake.insert(0, waker.clone());
            if this.2 {
                if shared.addr.is_null() {
                    Ready(())
                } else {
                    Pending
                }
            } else {
                *this.as_mut().pin_get_init() = true;
                shared.addr = unsafe { this.as_mut().pin_get_data().get_unchecked_mut() };
                for waker in shared.wake.drain(1..) {
                    waker.wake();
                }
                Pending
            }
        };
        this.0.0.mutex.unlock();
        output
    }
}

impl<T: Send> Drop for Message<T> {
    fn drop(&mut self) {
        self.0.0.mutex.lock();
        {
            let shared = unsafe { &mut *self.0.0.data.get() };
            if !self.2 || !shared.addr.is_null() {
                panic!("Message dropped without sending");
            }
        }
        self.0.0.mutex.unlock();
    }
}
