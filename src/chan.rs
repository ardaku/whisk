use alloc::{sync::Arc, vec::Vec};
use core::{
    cell::UnsafeCell,
    future::Future,
    mem::ManuallyDrop,
    pin::Pin,
    ptr::{self, NonNull},
    task::{
        Context,
        Poll::{self, Pending, Ready},
        Waker,
    },
};

use crate::spin::SpinLock;

#[derive(Debug)]
struct Shared<T: Send> {
    /// Address of data being sent
    addr: Option<NonNull<T>>,
    /// Wakers
    wake: Vec<Waker>,
}

impl<T: Send> Default for Shared<T> {
    fn default() -> Self {
        let addr = None;
        let wake = Vec::new();

        Self { addr, wake }
    }
}

unsafe impl<T: Send> Send for Shared<T> {}

/// A `Channel` notifies when another `Channel` sends a message.
///
/// Implemented as a rendezvous multi-producer/multi-consumer queue
#[derive(Debug)]
pub struct Channel<T: Send>(Arc<SpinLock<Shared<T>>>);

impl<T: Send> Clone for Channel<T> {
    fn clone(&self) -> Self {
        Channel(Arc::clone(&self.0))
    }
}

impl<T: Send> Default for Channel<T> {
    fn default() -> Self {
        Self(Arc::new(SpinLock::default()))
    }
}

impl<T: Send> Channel<T> {
    /// Create a new channel.
    pub fn new() -> Self {
        Self::default()
    }

    /// Send a message on this channel.
    #[inline(always)]
    pub fn send(&self, message: T) -> Message<T> {
        Message(
            (*self).clone(),
            ManuallyDrop::new(UnsafeCell::new(message)),
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
        let waker = cx.waker().clone();
        self.0.with(|shared| {
            if let Some(ptr) = shared.addr {
                let output = unsafe { ptr::read(ptr.as_ref()) };
                shared.addr = None;
                for waker in shared.wake.drain(..) {
                    waker.wake();
                }
                Ready(output)
            } else {
                // If a sender hasn't sent, return not ready
                shared.wake.push(waker);
                Pending
            }
        })
    }
}

/// A message in the process of being sent over a [`Channel`].
#[derive(Debug)]
pub struct Message<T: Send>(Channel<T>, ManuallyDrop<UnsafeCell<T>>, bool);

impl<T: Send> Message<T> {
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
        let waker = cx.waker().clone();
        this.0.clone().0.with(|shared| {
            if this.2 {
                if shared.addr.is_none() {
                    Ready(())
                } else {
                    shared.wake.push(waker);
                    Pending
                }
            } else {
                *this.as_mut().pin_get_init() = true;
                shared.addr = NonNull::new(this.1.get());
                shared.wake.push(waker);
                for waker in shared.wake.drain(..shared.wake.len() - 1) {
                    waker.wake();
                }
                Pending
            }
        })
    }
}
