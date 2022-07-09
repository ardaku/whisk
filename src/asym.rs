use alloc::boxed::Box;
use core::{
    future::Future,
    marker::PhantomData,
    mem::{self, MaybeUninit},
    pin::Pin,
    ptr::{self, NonNull},
    sync::atomic::{
        AtomicPtr,
        Ordering::{Acquire, Release},
    },
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
#[must_use = "Sender should send a message before being dropped"]
pub struct Sender<T: Send>(NonNull<Channel>, PhantomData<*mut T>);

unsafe impl<T: Send> Send for Sender<T> {}

impl<T: Send> Sender<T> {
    /// Send a message
    #[inline]
    pub(crate) fn send_and_reuse(&self, mut message: T) {
        let ptr: *mut _ = &mut message;

        unsafe {
            // Spin loop until receiver lock is released
            let msg = loop {
                let p = (*self.0.as_ptr()).msg.swap(ptr::null_mut(), Acquire);
                if !p.is_null() {
                    break p;
                }
                core::hint::spin_loop();
            };

            // Send data
            *msg.cast() = message;

            // Release lock (pointer unused)
            (*self.0.as_ptr()).msg.store(ptr.cast(), Release);

            // Wake Receiver
            (*self.0.as_ptr()).waker.wake_by_ref();
        }
    }

    /// Send a message
    #[inline]
    pub fn send(self, message: T) {
        self.send_and_reuse(message);
        // mem::forget(self);
    }
}

/// Receives a single message (oneshot-rendezvous channel)
///
/// Created from a [`Channel`] context.
#[derive(Debug)]
#[must_use = "Receiver must receive a message before being dropped"]
pub struct Receiver<T: Send>(NonNull<Channel>, PhantomData<*mut T>);

unsafe impl<T: Send> Send for Receiver<T> {}

impl<T: Send> Receiver<T> {
    #[inline]
    pub(crate) unsafe fn unuse(self) {
        Box::from_raw(self.0.as_ptr());
        mem::forget(self);
    }

    #[inline]
    pub(crate) async fn recv_and_reuse(&self) -> T {
        let clone = Receiver(self.0, PhantomData);
        let (val, chan) = clone.recv_chan().await;
        core::mem::forget(chan);
        val
    }

    /// Consume the receiver and receive the message
    #[inline]
    pub async fn recv(self) -> T {
        self.recv_chan().await.0
    }

    /// Consume the receiver and receive the message, plus the channel.
    #[inline]
    pub async fn recv_chan(self) -> (T, Box<Channel>) {
        let mut output = MaybeUninit::<T>::uninit();

        let mut future = Fut(self.0, output.as_mut_ptr().cast());
        // Release receiver lock
        unsafe {
            (*self.0.as_ptr()).msg.store(future.1, Release);
        }
        // Wait
        let chan = (&mut future).await;
        // Forget
        mem::forget(self);
        // Can safely assume init
        unsafe { (output.assume_init(), chan) }
    }
}

impl<T: Send> Drop for Receiver<T> {
    fn drop(&mut self) {
        panic!("Receiver dropped without receiving");
    }
}

struct Fut(NonNull<Channel>, *mut ());

impl Future for &mut Fut {
    type Output = Box<Channel>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            // If already locked by sender, spinlock until send complete.
            let addr = loop {
                let p = (*self.0.as_ptr()).msg.swap(ptr::null_mut(), Acquire);
                if !p.is_null() {
                    break p;
                }
            };

            if addr == self.1 {
                // Write has not completed, update waker
                (*self.0.as_ptr()).waker = cx.waker().clone();

                // Release spinlock
                (*self.0.as_ptr()).msg.store(addr, Release);

                Pending
            } else {
                // Write is complete, can safely assume init
                Ready(Box::from_raw(self.0.as_ptr()))
            }
        }
    }
}

/// Channel context
#[derive(Debug)]
pub struct Channel {
    waker: Waker,
    msg: AtomicPtr<()>,
}

unsafe impl Send for Channel {}

impl Channel {
    /// Create an asynchronous oneshot-rendezvous channel
    #[inline]
    pub fn pair<T: Send>() -> (Sender<T>, Receiver<T>) {
        let channel = Self {
            waker: coma(),
            msg: ptr::null_mut::<()>().into(),
        };
        let channel = Box::leak(Box::new(channel)).into();

        (Sender(channel, PhantomData), Receiver(channel, PhantomData))
    }

    /// Reuse the context to avoid extra allocation
    #[inline]
    pub fn to_pair<T: Send>(mut self: Box<Self>) -> (Sender<T>, Receiver<T>) {
        self.waker = coma();

        let channel = Box::leak(self).into();

        (Sender(channel, PhantomData), Receiver(channel, PhantomData))
    }
}

/// Create a waker that doesn't do anything (purposefully)
#[inline]
fn coma() -> Waker {
    #[inline]
    const unsafe fn dont(_: *const ()) {}

    #[inline]
    const unsafe fn coma(ptr: *const ()) -> RawWaker {
        RawWaker::new(ptr, &COMA)
    }

    const COMA: RawWakerVTable = RawWakerVTable::new(coma, dont, dont, dont);

    unsafe { Waker::from_raw(coma(ptr::null())) }
}
