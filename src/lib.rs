//! #### Simple and fast async channels
//! Simple and fast async channels that can be used to implement futures,
//! streams, notifiers, and actors.
//!
//! # Optional Features
//!  - **futures-core**: Implement [`Stream`](futures_core::Stream) for
//!    [`Channel`] (generic `T` must be `Option<Item>`)
//!  - **pasts**: Implement [`Notifier`](pasts::Notifier) for [`Channel`]
//!
//! # Getting Started
//!
//! ```rust
//! use whisk::Channel;
//!
//! enum Cmd {
//!     /// Tell messenger to add
//!     Add(u32, u32, Channel<u32>),
//! }
//!
//! async fn worker_main(mut channel: Channel<Option<Cmd>>) {
//!     while let Some(command) = channel.recv().await {
//!         println!("Worker receiving command");
//!         match command {
//!             Cmd::Add(a, b, s) => {
//!                 s.send(a + b).await;
//!             }
//!         }
//!     }
//!
//!     println!("Worker stopping…");
//! }
//!
//! async fn tasker_main() {
//!     // Create worker on new thread
//!     println!("Spawning worker…");
//!     let channel = Channel::new();
//!     let worker_thread = {
//!         let channel = channel.clone();
//!         std::thread::spawn(move || {
//!             pasts::Executor::default()
//!                 .spawn(Box::pin(async move { worker_main(channel).await }))
//!         })
//!     };
//!
//!     // Do an addition
//!     println!("Sending command…");
//!     let oneshot = Channel::new();
//!     channel.send(Some(Cmd::Add(43, 400, oneshot.clone()))).await;
//!     println!("Receiving response…");
//!     let response = oneshot.await;
//!     assert_eq!(response, 443);
//!
//!     // Tell worker to stop
//!     println!("Stopping worker…");
//!     channel.send(None).await;
//!     println!("Waiting for worker to stop…");
//!
//!     worker_thread.join().unwrap();
//!     println!("Worker thread joined");
//! }
//!
//! # #[ntest::timeout(1000)]
//! // Call into executor of your choice
//! fn main() {
//!     pasts::Executor::default().spawn(Box::pin(tasker_main()))
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
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::{self, Arc};
use core::{
    cell::UnsafeCell,
    future::Future,
    mem::MaybeUninit,
    pin::Pin,
    sync::atomic::{
        self, AtomicBool,
        Ordering::{Acquire, Relaxed, Release},
    },
    task::{
        Context,
        Poll::{self, Pending, Ready},
        Waker,
    },
};

#[allow(unsafe_code)]
mod list {
    use super::*;

    /// A list
    #[derive(Debug)]
    pub(super) struct List<T, const N: usize> {
        data: [MaybeUninit<T>; N],
        size: usize,
    }

    impl<T, const N: usize> List<T, N> {
        #[inline]
        pub(super) fn new() -> Self {
            let data = unsafe {
                MaybeUninit::<[MaybeUninit<T>; N]>::uninit().assume_init()
            };
            let size = 0;

            Self { data, size }
        }

        #[inline(always)]
        pub(super) fn push(&mut self, item: T, idx: usize) -> usize {
            let idx = if idx == usize::MAX { self.size } else { idx };
            self.data[idx] = MaybeUninit::new(item);
            self.size += 1;
            idx
        }

        #[inline(always)]
        pub(super) fn drain(&mut self) -> impl Iterator<Item = T> + '_ {
            let mut size = 0;
            (size, self.size) = (self.size, size);
            self.data
                .iter()
                .take(size)
                .map(|t| unsafe { t.assume_init_read() })
        }
    }
}

#[allow(unsafe_code)]
mod spin {
    use super::*;

    /// A spinlock
    #[derive(Debug, Default)]
    pub(super) struct Spin<T: Default> {
        flag: AtomicBool,
        data: UnsafeCell<T>,
    }

    impl<T: Default> Spin<T> {
        #[inline(always)]
        pub(super) fn with<O>(&self, then: impl FnOnce(&mut T) -> O) -> O {
            while self
                .flag
                .compare_exchange_weak(false, true, Relaxed, Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }
            atomic::fence(Acquire);
            let output = then(unsafe { &mut *self.data.get() });
            self.flag.store(false, Release);
            output
        }
    }

    unsafe impl<T: Default + Send> Send for Spin<T> {}
    unsafe impl<T: Default + Send> Sync for Spin<T> {}
}

#[derive(Debug)]
struct Locked<T: Send, const S: usize, const R: usize> {
    /// Receive wakers
    recv: list::List<Waker, R>,
    /// Send wakers
    send: list::List<Waker, S>,
    /// Data in transit
    data: Option<T>,
}

impl<T: Send, const S: usize, const R: usize> Default for Locked<T, S, R> {
    #[inline]
    fn default() -> Self {
        let data = None;
        let send = list::List::new();
        let recv = list::List::new();

        Self { data, send, recv }
    }
}

#[derive(Debug, Default)]
struct Shared<T: Send, const S: usize, const R: usize> {
    spin: spin::Spin<Locked<T, S, R>>,
}

/// A `Channel` notifies when another `Channel` sends a message.
///
/// Implemented as a multi-producer/multi-consumer queue of size 1.
///
/// Const generic `S` is the upper bound on the number of channels that can be
/// sending at once (doesn't include inactive channels).
///
/// Const generic `R` is the upper bound on the number of channels that can be
/// receiving at once (doesn't include inactive channels).
///
/// Enable the **`futures-core`** feature for `Channel` to implement
/// [`Stream`](futures_core::Stream) (generic `T` must be `Option<Item>`).
///
/// Enable the **`pasts`** feature for `Channel` to implement
/// [`Notifier`](pasts::Notifier).
#[derive(Debug)]
pub struct Channel<T: Send + Unpin, const S: usize = 1, const R: usize = 1>(
    Arc<Shared<T, S, R>>,
    usize,
);

impl<T, const S: usize, const R: usize> Clone for Channel<T, S, R>
where
    T: Send + Unpin,
{
    #[inline]
    fn clone(&self) -> Self {
        Channel(Arc::clone(&self.0), usize::MAX)
    }
}

impl<T, const S: usize, const R: usize> Default for Channel<T, S, R>
where
    T: Send + Unpin,
{
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + Unpin, const S: usize, const R: usize> Channel<T, S, R> {
    /// Create a new channel.
    #[inline]
    pub fn new() -> Self {
        let spin = spin::Spin::default();

        Self(Arc::new(Shared { spin }), usize::MAX)
    }

    /// Send a message on this channel.
    #[inline(always)]
    pub fn send(&self, message: T) -> impl Future<Output = ()> + Send + Unpin {
        let mut chan = (*self).clone();
        chan.1 = usize::MAX;
        Message(chan, Some(message))
    }

    /// Receive a message from this channel.
    #[inline(always)]
    pub fn recv(
        &mut self,
    ) -> impl Future<Output = T> + Send + Sync + Unpin + '_ {
        self
    }

    /// Create a new corresponding [`Weak`] channel.
    #[inline]
    pub fn downgrade(&self) -> Weak<T, S, R> {
        Weak(Arc::downgrade(&self.0))
    }
}

impl<T, const S: usize, const R: usize> Future for Channel<T, S, R>
where
    T: Send + Unpin,
{
    type Output = T;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let waker = cx.waker();
        this.0.spin.with(|shared| {
            if let Some(output) = shared.data.take() {
                for waker in shared.send.drain() {
                    waker.wake();
                }
                Ready(output)
            } else {
                this.1 = shared.recv.push(waker.clone(), this.1);
                Pending
            }
        })
    }
}

#[cfg(feature = "pasts")]
impl<T, const S: usize, const R: usize> pasts::Notifier for Channel<T, S, R>
where
    T: Send + Unpin,
{
    type Event = T;

    #[inline(always)]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.poll(cx)
    }
}

#[cfg(feature = "futures-core")]
impl<T, const S: usize, const R: usize> futures_core::Stream
    for Channel<Option<T>, S, R>
where
    T: Send + Unpin,
{
    type Item = T;

    #[inline(always)]
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.poll(cx)
    }
}

/// A weak version of a `Channel`.
#[derive(Debug, Default)]
pub struct Weak<T: Send + Unpin, const S: usize = 1, const R: usize = 1>(
    sync::Weak<Shared<T, S, R>>,
);

impl<T: Send + Unpin, const S: usize, const R: usize> Weak<T, S, R> {
    /// Calling `upgrade()` will always return `None`.
    #[inline]
    pub fn new() -> Self {
        Self(sync::Weak::new())
    }

    /// Attempt to upgrade the Weak channel to a [`Channel`].
    #[inline]
    pub fn upgrade(&self) -> Option<Channel<T, S, R>> {
        Some(Channel(self.0.upgrade()?, usize::MAX))
    }
}

/// A message in the process of being sent over a [`Channel`].
#[derive(Debug)]
struct Message<T: Send + Unpin, const S: usize, const R: usize>(
    Channel<T, S, R>,
    Option<T>,
);

impl<T, const S: usize, const R: usize> Future for Message<T, S, R>
where
    T: Send + Unpin,
{
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let waker = cx.waker();
        this.0 .0.spin.with(|shared| {
            if shared.data.is_none() {
                shared.data = this.1.take();
                for waker in shared.recv.drain() {
                    waker.wake();
                }
                Ready(())
            } else {
                this.0 .1 = shared.send.push(waker.clone(), this.0 .1);
                Pending
            }
        })
    }
}

impl<T, const S: usize, const R: usize> Drop for Message<T, S, R>
where
    T: Send + Unpin,
{
    fn drop(&mut self) {
        if self.1.is_some() {
            panic!("Message dropped without sending");
        }
    }
}
