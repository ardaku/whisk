//! #### Simple and fast async channels
//! Whisk provides oneshot-rendezvous and spsc channels that can be used to
//! implement futures, streams, notifiers, and actors.
//!
//! # Optional Features
//! The `std` feature is enabled by default, disable it to use on **no_std**.
//!
//! # Getting Started
//!
//! ```rust
//! use whisk::{Channel, Sender, Tasker, Worker};
//!
//! enum Cmd {
//!     /// Tell messenger to add
//!     Add(u32, u32, Sender<u32>),
//! }
//!
//! async fn worker(tasker: Tasker<Cmd>) {
//!     while let Some(command) = tasker.recv_next().await {
//!         println!("Worker receiving command");
//!         match command {
//!             Cmd::Add(a, b, s) => s.send(a + b).await,
//!         }
//!     }
//!
//!     println!("Worker stopping…");
//! }
//!
//! async fn tasker() {
//!     // Create worker on new thread
//!     println!("Spawning worker…");
//!     let mut worker_thread = None;
//!     let worker = Worker::new(|tasker| {
//!         worker_thread = Some(std::thread::spawn(move || {
//!             pasts::Executor::default()
//!                 .spawn(Box::pin(async move { worker(tasker).await }))
//!         }));
//!     });
//!
//!     // Do an addition
//!     println!("Sending command…");
//!     let (send, recv) = Channel::pair();
//!     worker.send(Cmd::Add(43, 400, send)).await;
//!     println!("Receiving response…");
//!     let response = recv.recv().await;
//!     assert_eq!(response, 443);
//!
//!     // Tell worker to stop
//!     println!("Stopping worker…");
//!     worker.stop().await;
//!     println!("Waiting for worker to stop…");
//!
//!     worker_thread.unwrap().join().unwrap();
//!     println!("Worker thread joined");
//! }
//!
//! # #[ntest::timeout(1000)]
//! // Call into executor of your choice
//! fn main() {
//!     pasts::Executor::default().spawn(Box::pin(tasker()))
//! }
//! ```

#![no_std]
#![doc(
    html_logo_url = "https://ardaku.github.io/mm/logo.svg",
    html_favicon_url = "https://ardaku.github.io/mm/icon.svg",
    html_root_url = "https://docs.rs/whisk"
)]
#![warn(
    anonymous_parameters,
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    nonstandard_style,
    rust_2018_idioms,
    single_use_lifetimes,
    trivial_casts,
    trivial_numeric_casts,
    unreachable_pub,
    unused_extern_crates,
    unused_qualifications,
    variant_size_differences
)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

use alloc::boxed::Box;
use core::{
    future::Future,
    marker::PhantomData,
    mem::{self, MaybeUninit, ManuallyDrop},
    pin::Pin,
    ptr::{self, NonNull},
    sync::atomic::{
        AtomicBool, AtomicPtr,
        Ordering::{Acquire, Release},
    },
    task::{
        Context, Poll,
        Poll::{Pending, Ready},
        Waker,
    },
};

mod seal {
    use core::marker::PhantomData;

    #[derive(Default, Debug)]
    pub struct For<T>(PhantomData<*mut T>);

    impl<T> For<T> {
        #[inline]
        pub(super) fn new() -> Self {
            For(PhantomData)
        }
    }
}

use seal::For;

/// Sends a single message (oneshot-rendezvous channel)
///
/// Created from a [`Channel`] context.
#[derive(Debug)]
#[must_use = "Sender should send a message before being dropped"]
pub struct Sender<T: Send + Unpin, U = For<T>> {
    channel: NonNull<Channel>,
    message: ManuallyDrop<U>,
    _for: For<T>,
}

unsafe impl<T: Send + Unpin> Send for Sender<T> {}

impl<T: Send + Unpin> Sender<T> {
    /// Send a message
    #[inline]
    pub(crate) async fn send_and_reuse(&self, mut message: T) {
        let ptr: *mut _ = &mut message;

        unsafe {
            let mut msg = ptr::null_mut();
            for _ in 0..8 {
                msg =
                    (*self.channel.as_ptr()).msg.swap(ptr::null_mut(), Acquire);
                if !msg.is_null() {
                    break;
                }
                core::hint::spin_loop();
            }

            // Wait until receive requested
            if msg.is_null() {
                Sender { channel: self.channel, message: ManuallyDrop::new(message), _for: For::new() }.await;
            } else {
                // Send data
                *msg.cast() = message;
            }

            // Read waker before allowing to be free'd
            let waker = (*self.channel.as_ptr()).waker.take();

            // Release lock (pointer unused)
            (*self.channel.as_ptr()).msg.store(ptr.cast(), Release);

            // Wake Receiver
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    /// Send a message
    #[inline]
    pub async fn send(self, message: T) {
        self.send_and_reuse(message).await;
    }
}

impl<T: Send + Unpin> Sender<T, T> {
    #[inline]
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let channel = self.channel.as_ptr();

        unsafe {
            // Lock other waker
            while (*channel).lock.swap(true, Acquire) {
                core::hint::spin_loop();
            }

            // Check ready status
            let ptr = (*channel).msg.swap(ptr::null_mut(), Acquire);
            let pending = ptr.is_null();

            // Set other waker
            let waker = cx.waker().clone();
            (*channel).other = pending.then(move || waker);

            // Release lock on other waker
            (*channel).lock.store(false, Release);

            if pending {
                return Pending;
            }

            *ptr.cast() = ManuallyDrop::take(&mut self.message);
            Ready(())
        }
    }
}

impl<T: Send + Unpin> Future for Sender<T, T> {
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.poll_next(cx)
    }
}

/// Receives a single message (oneshot-rendezvous channel)
///
/// Created from a [`Channel`] context.
#[derive(Debug)]
#[must_use = "Receiver must receive a message before being dropped"]
pub struct Receiver<T: Send + Unpin>(NonNull<Channel>, PhantomData<*mut T>);

unsafe impl<T: Send + Unpin> Send for Receiver<T> {}

impl<T: Send + Unpin> Receiver<T> {
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

        let future = Fut(self.0, output.as_mut_ptr().cast());
        // Release receiver lock
        unsafe {
            (*self.0.as_ptr()).msg.store(future.1, Release);
        }
        // Wait
        let chan = future.await;
        // Forget
        mem::forget(self);
        // Can safely assume init
        unsafe { (output.assume_init(), chan) }
    }
}

impl<T: Send + Unpin> Drop for Receiver<T> {
    #[inline]
    fn drop(&mut self) {
        panic!("Receiver dropped without receiving");
    }
}

struct Fut(NonNull<Channel>, *mut ());

impl Future for Fut {
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
                core::hint::spin_loop();
            };

            if addr == self.1 {
                // Write has not completed, update waker
                (*self.0.as_ptr()).waker = Some(cx.waker().clone());

                // Release spinlock
                (*self.0.as_ptr()).msg.store(addr, Release);

                // Lock other waker
                while (*self.0.as_ptr()).lock.swap(true, Acquire) {
                    core::hint::spin_loop();
                }

                // Wake other waker
                if let Some(w) = (*self.0.as_ptr()).other.take() {
                    w.wake()
                }

                // Release lock on other waker
                (*self.0.as_ptr()).lock.store(false, Release);

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
    waker: Option<Waker>,
    other: Option<Waker>,
    msg: AtomicPtr<()>,
    lock: AtomicBool,
}

unsafe impl Send for Channel {}

impl Channel {
    /// Create an asynchronous oneshot-rendezvous channel
    #[inline]
    pub fn pair<T: Send + Unpin>() -> (Sender<T>, Receiver<T>) {
        let channel = Self {
            waker: None,
            other: None,
            msg: ptr::null_mut::<()>().into(),
            lock: false.into(),
        };
        let channel = Box::leak(Box::new(channel)).into();

        (
            Sender {
                channel,
                message: ManuallyDrop::new(For::new()),
                _for: For::new(),
            },
            Receiver(channel, PhantomData),
        )
    }

    /// Reuse the context to avoid extra allocation
    #[inline]
    pub fn to_pair<T: Send + Unpin>(self: Box<Self>) -> (Sender<T>, Receiver<T>) {
        let channel = Box::leak(self).into();

        (
            Sender {
                channel,
                message: ManuallyDrop::new(For::new()),
                _for: For::new(),
            },
            Receiver(channel, PhantomData),
        )
    }
}

/// Handle to a worker - command consumer (spsc-rendezvous channel)
#[derive(Debug)]
pub struct Worker<T: Send + Unpin>(Sender<Option<T>>);

impl<T: Send + Unpin> Worker<T> {
    /// Start up a worker (similar to the actor concept).
    #[inline]
    pub fn new(cb: impl FnOnce(Tasker<T>)) -> Self {
        let (sender, receiver) = Channel::pair();

        // Launch worker
        cb(Tasker(core::cell::Cell::new(Some(receiver))));

        // Return worker handle
        Self(sender)
    }

    /// Send a command to the worker.
    #[inline]
    pub async fn send(&self, cmd: T) {
        self.0.send_and_reuse(Some(cmd)).await;
    }

    /// Stop the worker.
    #[inline]
    pub async fn stop(self) {
        self.0.send_and_reuse(None).await;
        core::mem::forget(self);
    }
}

impl<T: Send + Unpin> Drop for Worker<T> {
    #[inline]
    fn drop(&mut self) {
        panic!("Worker dropped without stopping");
    }
}

/// Handle to a tasker - command producer (spsc-rendezvous channel)
pub struct Tasker<T: Send + Unpin>(core::cell::Cell<Option<Receiver<Option<T>>>>);

impl<T: Send + core::fmt::Debug + Unpin> core::fmt::Debug for Tasker<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let tmp = self.0.take();
        f.debug_struct("Foo").field("0", &tmp).finish()?;
        self.0.set(tmp);
        Ok(())
    }
}

impl<T: Send + Unpin> Tasker<T> {
    /// Get the next command from the tasker, returns [`None`] on stop
    #[inline]
    pub async fn recv_next(&self) -> Option<T> {
        let recver = self.0.replace(None)?;

        if let Some(value) = recver.recv_and_reuse().await {
            self.0.set(Some(recver));
            Some(value)
        } else {
            unsafe { recver.unuse() };
            None
        }
    }
}

impl<T: Send + Unpin> Drop for Tasker<T> {
    #[inline]
    fn drop(&mut self) {
        if self.0.take().is_some() {
            panic!("Tasker dropped before Worker");
        }
    }
}
