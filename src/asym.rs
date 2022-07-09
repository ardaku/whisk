use alloc::boxed::Box;
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::atomic::{
        AtomicBool, AtomicPtr,
        Ordering::{AcqRel, Acquire, Relaxed, Release},
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
pub struct Sender<T: Send>(*mut Channel, PhantomData<*mut T>);

unsafe impl<T: Send> Send for Sender<T> {}

impl<T: Send> Sender<T> {
    /// Send a message
    #[inline]
    pub(crate) fn send_and_reuse(&self, message: T) {
        // Inhibit drop because we're transferring data
        let mut message = core::mem::ManuallyDrop::new(message);

        unsafe {
            // Set address for reading
            let addr: *mut _ = &mut *message;
            (*self.0).msg = AtomicPtr::from(addr.cast());
            // Spin lock until Waker is updated
            while (*self.0)
                .lock
                .compare_exchange_weak(false, true, AcqRel, Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }
            // Wake by reference
            (*self.0).waker.wake_by_ref();
            // Once awoken, spin lock until message is sent successfully
            while (*self.0)
                .lock
                .compare_exchange_weak(false, true, AcqRel, Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }
        }
    }

    /// Send a message
    #[inline]
    pub fn send(self, message: T) {
        self.send_and_reuse(message)
    }
}

/// Receives a single message (oneshot-rendezvous channel)
///
/// Created from a [`Channel`] context.
#[derive(Debug)]
pub struct Receiver<T: Send>(*mut Channel, PhantomData<*mut T>);

unsafe impl<T: Send> Send for Receiver<T> {}

impl<T: Send> Receiver<T> {
    #[inline]
    pub(crate) unsafe fn unuse(&self) {
        Box::from_raw(self.0);
    }

    #[inline]
    pub(crate) async fn recv_and_reuse(&self) -> T {
        let clone = Receiver(self.0, PhantomData);
        let (val, chan) = clone.await;
        core::mem::forget(chan);
        val
    }

    /// Consume the receiver and receive the message
    #[inline]
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
            if (*self.0).lock.fetch_or(true, Acquire) {
                let message: T =
                    core::ptr::read((*(*self.0).msg.get_mut()).cast());
                (*self.0).lock.store(false, Release);
                // FIXME: Move to drop of Channel by adding InnerChannel
                // spinlock to allow box to be dropped
                while (*self.0)
                    .lock
                    .compare_exchange_weak(true, false, AcqRel, Relaxed)
                    .is_err()
                {
                    core::hint::spin_loop();
                }
                Ready((message, Box::from_raw(self.0)))
            } else {
                (*self.0).waker = cx.waker().clone();
                (*self.0).lock.store(false, Release);
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
    msg: AtomicPtr<()>,
}

unsafe impl Send for Channel {}

impl Channel {
    /// Create an asynchronous oneshot-rendezvous channel
    #[inline]
    pub fn pair<T: Send>() -> (Sender<T>, Receiver<T>) {
        let mut channel = Self {
            lock: false.into(),
            waker: coma(),
            msg: core::ptr::null_mut::<()>().into(),
        };
        // Non-null unused junk pointer
        channel.msg = core::ptr::addr_of_mut!(channel).cast::<()>().into();
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

#[inline]
const unsafe fn do_nothing(_: *const ()) {}

#[inline]
const unsafe fn get_coma(ptr: *const ()) -> RawWaker {
    RawWaker::new(ptr, &COMA)
}

const COMA: RawWakerVTable =
    RawWakerVTable::new(get_coma, do_nothing, do_nothing, do_nothing);

/// Create a waker that doesn't do anything (purposefully)
#[inline]
fn coma() -> Waker {
    unsafe { Waker::from_raw(get_coma(core::ptr::null())) }
}
