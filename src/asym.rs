use alloc::boxed::Box;
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{
        Context, Poll,
        Poll::{Pending, Ready},
        RawWaker, RawWakerVTable, Waker,
    },
};

/// Sends a single message (oneshot-rendezvous channel)
///
/// Created from a [`Channel`] context.
#[derive(Debug)]
pub struct Sender<T: Send>(*mut Channel, PhantomData<*mut T>);

impl<T: Send> Sender<T> {
    /// Send a message
    #[inline]
    pub fn send(self, message: T) {
        // Inhibit drop because we're transferring data
        let message = core::mem::ManuallyDrop::new(message);
        // Safety: never moved because shadowed
        let message = unsafe { Pin::<&_>::new_unchecked(&message) };
        // Manually drop after message is sent
        let mut message = core::mem::ManuallyDrop::new(message);

        unsafe {
            // Set address for reading
            let addr: *const _ = (*message).get_ref();
            (*self.0).msg = addr.cast();
            // Spin lock until Waker is updated
            while (*self.0).lock.fetch_or(true, Ordering::Acquire) {
                core::hint::spin_loop();
            }
            // Now that it's updated, send wake event
            //
            // Safety (guaranteed by `Box::leak(Box::new(t))`):
            //  - pointer is guaranteed to be valid
            //  - pointer is aligned
            //  - pointer is initialized
            core::ptr::read(&(*self.0).waker).wake();
            // Once awoken, spin lock until message is sent successfully
            while (*self.0).lock.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
            // Delay forget of pinned reference until sent over channel
            core::mem::ManuallyDrop::drop(&mut message);
        }
    }
}

/// Receives a single message (oneshot-rendezvous channel)
///
/// Created from a [`Channel`] context.
#[derive(Debug)]
pub struct Receiver<T: Send>(*mut Channel, PhantomData<*mut T>);

impl<T: Send> Receiver<T> {
    /// Consume the receiver and receive the message
    pub async fn recv(self) -> T {
        self.await.0
    }
}

impl<T: Send> Future for Receiver<T> {
    type Output = (T, Box<Channel>);

    #[inline]
    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        unsafe {
            if (*self.0).lock.fetch_or(true, Ordering::Acquire) {
                let message: T = core::ptr::read((*self.0).msg.cast());
                (*self.0).lock.store(false, Ordering::Release);
                Ready((message, Box::from_raw(self.0)))
            } else {
                (*self.0).waker = cx.waker().clone();
                (*self.0).lock.store(false, Ordering::Release);
                Pending
            }
        }
    }
}

/// Channel context
#[derive(Debug)]
pub struct Channel {
    lock: AtomicBool,
    waker: Waker,
    msg: *const (),
}

impl Channel {
    /// Create an asynchronous oneshot-rendezvous channel
    #[inline]
    pub fn pair<T: Send>() -> (Sender<T>, Receiver<T>) {
        let channel = Self {
            lock: false.into(),
            waker: coma(),
            msg: core::ptr::null(),
        };
        let channel = Box::leak(Box::new(channel));

        (Sender(channel, PhantomData), Receiver(channel, PhantomData))
    }

    /// Reuse the context to avoid extra allocation
    #[inline]
    pub fn to_pair<T: Send>(mut self: Box<Self>) -> (Sender<T>, Receiver<T>) {
        self.waker = coma();

        let channel = Box::leak(self);

        (Sender(channel, PhantomData), Receiver(channel, PhantomData))
    }
}

/// Create a waker that doesn't do anything (purposefully)
#[inline]
fn coma() -> Waker {
    unsafe fn do_nothing(_: *const ()) {}

    unsafe fn coma(_: *const ()) -> RawWaker {
        RawWaker::new(core::ptr::null(), &COMA)
    }

    const COMA: RawWakerVTable =
        RawWakerVTable::new(coma, do_nothing, do_nothing, do_nothing);

    unsafe { Waker::from_raw(coma(core::ptr::null())) }
}
