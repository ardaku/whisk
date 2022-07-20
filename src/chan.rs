//! Sender/Receiver Channel implementation

use alloc::boxed::Box;
use core::{
    cell::{Cell, UnsafeCell},
    future::Future,
    mem::{ManuallyDrop, MaybeUninit},
    pin::Pin,
    ptr,
    sync::atomic::{
        AtomicBool, AtomicPtr,
        Ordering::{Acquire, Release},
    },
    task::{
        Context,
        Poll::{self, Pending, Ready},
        Waker,
    },
};

use crate::{
    spin::SpinLock,
    spsc::{Tasker, Worker},
};

/// Rendezvous barrier
#[derive(Default, Debug)]
struct Barrier {
    /// Waker for sender future
    send_waker: SpinLock<Waker>,
    /// Waker for receiver future
    recv_waker: SpinLock<Waker>,
    /// Sender future started
    send_ready: AtomicBool,
    /// Receiver future started
    recv_ready: AtomicBool,
}

impl Barrier {
    /// Sender wake fn
    fn wake_next_send(&self) {
        self.send_ready.store(true, Release);
        if let Some(waker) = self.recv_waker.take() {
            // FIXME: try-take opt
            waker.wake();
        }
    }

    /// Sender poll fn
    fn poll_next_send(&self, cx: &mut Context<'_>) -> Poll<()> {
        self.send_waker.store(Some(cx.waker().clone())); // FIXME: try-store?
        if self.recv_ready.load(Acquire) {
            Ready(())
        } else {
            Pending
        }
    }

    /// Receiver wake fn
    fn wake_next_recv(&self) {
        self.recv_ready.store(true, Release);
        if let Some(waker) = self.send_waker.take() {
            // FIXME: try-take opt
            waker.wake();
        }
    }

    /// Receiver poll fn (Wait for send ready before copying)
    fn poll_next_recv(&self, cx: &mut Context<'_>) -> Poll<()> {
        self.recv_waker.store(Some(cx.waker().clone())); // FIXME: try-store?
        if self.send_ready.load(Acquire) {
            Ready(())
        } else {
            Pending
        }
    }
}

/// A reuseable channel
#[derive(Debug)]
pub struct Channel<T: Send> {
    /// Barrier to synchronize tasks
    barrier: Barrier,
    /// Message passing mechanism
    ///  1. Init and set to `null()`
    ///  2. Sender init -> Set pinned src pointer
    ///  3. Sender wake receiver (barrier point A)
    ///  4. Receiver copy to dest
    ///  5. Receiver wake sender (barrier point B)
    message: AtomicPtr<T>,
}

impl<T: Send> Channel<T> {
    /// Create a reusable channel.
    pub fn new() -> Box<Self> {
        Self {
            barrier: Barrier::default(),
            message: AtomicPtr::default(),
        }
        .into()
    }

    /// Convert into oneshot sender/receiver handles
    pub fn oneshot(self: Box<Self>) -> (Sender<T>, Receiver<T>) {
        let chan = Box::leak(self);
        (Sender(chan), Receiver(chan))
    }

    /// Re-use
    pub(crate) fn reuse(&mut self) {
        self.barrier.send_ready = AtomicBool::new(false);
        self.barrier.recv_ready = AtomicBool::new(false);
        self.message = AtomicPtr::default();
    }
}

impl<T: Send> Channel<Option<T>> {
    /// Convert into spsc worker/tasker handles
    pub fn spsc(self: Box<Self>) -> (Worker<T>, Tasker<T>) {
        let (sender, receiver) = self.oneshot();
        (Worker(sender), Tasker(receiver.recv()))
    }
}

/// Oneshot sender
#[derive(Debug)]
pub struct Sender<T: Send>(pub(crate) *mut Channel<T>);

unsafe impl<T: Send> Send for Sender<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}

impl<T: Send> Sender<T> {
    /// Send a message
    pub fn send(self, value: T) -> Sending<T> {
        let channel = self.0;
        core::mem::forget(self);
        Sending(
            Cell::new(channel),
            UnsafeCell::new(ManuallyDrop::new(value)),
        )
    }
}

impl<T: Send> Drop for Sender<T> {
    fn drop(&mut self) {
        panic!("Dropped oneshot without sending");
    }
}

/// Oneshot receiver
#[derive(Debug)]
pub struct Receiver<T: Send>(*mut Channel<T>);

unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Sync for Receiver<T> {}

impl<T: Send> Receiver<T> {
    /// Receive from `Sender`.
    pub fn recv(self) -> Receiving<T> {
        let channel = self.0;
        core::mem::forget(self);
        Receiving(channel)
    }
}

impl<T: Send> Drop for Receiver<T> {
    fn drop(&mut self) {
        panic!("Dropped oneshot without receiving");
    }
}

/// Sender future / notifier / stream
#[derive(Debug)]
pub struct Sending<T: Send>(Cell<*mut Channel<T>>, UnsafeCell<ManuallyDrop<T>>);

unsafe impl<T: Send> Send for Sending<T> {}
unsafe impl<T: Send> Sync for Sending<T> {}

impl<T: Send> Sending<T> {
    /// Run an implementation of poll/poll_next for a fused future/notifier.
    pub fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Box<Channel<T>>> {
        let chan = self.0.get();
        if chan.is_null() {
            // Sending has already responded to request
            return Pending;
        }

        // Barrier point A
        if unsafe { (*chan).barrier.poll_next_send(cx).is_ready() } {
            // Reuse channel
            let mut channel = unsafe { Box::from_raw(self.0.get()) };
            // Take
            self.0.set(ptr::null_mut());
            // Reuse
            channel.reuse();
            // Done
            return Ready(channel);
        } else if unsafe { (*(*chan).message.get_mut()).is_null() } {
            // Receiving hasn't requested yet, set pointer
            unsafe { (*chan).message.store(self.1.get().cast(), Release) };
            // Wake
            unsafe { (*chan).barrier.wake_next_send() };
        }

        Pending
    }
}

impl<T: Send> Future for Sending<T> {
    type Output = Box<Channel<T>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_next(cx)
    }
}

impl<T: Send> Drop for Sending<T> {
    fn drop(&mut self) {
        if !self.0.get().is_null() {
            panic!("Sending dropped before sending");
        }
    }
}

/// Receiver future / notifier /stream
#[derive(Debug)]
pub struct Receiving<T: Send>(*mut Channel<T>);

unsafe impl<T: Send> Send for Receiving<T> {}
unsafe impl<T: Send> Sync for Receiving<T> {}

impl<T: Send> Receiving<T> {
    pub(crate) fn poll_next_reuse(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<T> {
        if self.0.is_null() {
            return Pending;
        }

        let ptr = unsafe { (*self.0).message.load(Acquire) };
        if ptr.is_null() {
            Pending
        } else if unsafe { (*self.0).barrier.poll_next_recv(cx).is_ready() } {
            let mut message = MaybeUninit::uninit();
            // Move
            unsafe { ptr::copy(ptr, message.as_mut_ptr(), 1) };
            // Wake: Barrier point B
            unsafe { (*self.0).barrier.wake_next_recv() };
            // Done
            Ready(unsafe { message.assume_init() })
        } else {
            Pending
        }
    }

    pub(crate) fn poll_next_unuse(mut self: Pin<&mut Self>) {
        self.0 = ptr::null_mut();
    }

    /// Run an implementation of poll/poll_next for a fused future/notifier.
    pub fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<T> {
        if let Ready(message) = self.as_mut().poll_next_reuse(cx) {
            // Prevent reuse
            self.poll_next_unuse();
            Ready(message)
        } else {
            Pending
        }
    }
}

impl<T: Send> Future for Receiving<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_next(cx)
    }
}

impl<T: Send> Drop for Receiving<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            panic!("Receiving dropped before receiving");
        }
    }
}
