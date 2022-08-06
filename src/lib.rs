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
//! async fn worker_main(channel: Channel<Option<Cmd>>) {
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

use alloc::{
    sync::{self, Arc},
    vec::Vec,
};
use core::{
    cell::{Cell, UnsafeCell},
    future::Future,
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
mod spin {
    use super::*;

    /// A spinlock
    #[derive(Default)]
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

/// Type for waking on send or receive
#[derive(Default)]
#[repr(C)]
struct Wake {
    /// Channel waker
    wake: Option<Waker>,
    /// Channel unique identifier (the arc pointer casted to usize)
    chan: usize,
    /// Heap wakers
    list: Vec<(usize, Waker)>,
}

impl Wake {
    /// Register a waker for a channel
    #[inline(always)]
    fn register(&mut self, chan: usize, waker: Waker) {
        if let Some(wake) = self.wake.take() {
            if self.chan == chan {
                (self.chan, self.wake) = (chan, Some(waker));
            } else {
                self.list.extend([(self.chan, wake), (chan, waker)]);
            }
        } else if self.list.is_empty() {
            (self.chan, self.wake) = (chan, Some(waker));
        } else if let Some(wake) = self.list.iter_mut().find(|w| w.0 == chan) {
            wake.1 = waker;
        } else {
            self.list.push((chan, waker));
        }
    }

    /// Wake all channels and de-register all wakers
    #[inline(always)]
    fn wake(&mut self) {
        if let Some(waker) = self.wake.take() {
            waker.wake();
            return;
        }
        for waker in self.list.drain(..) {
            waker.1.wake();
        }
    }
}

struct Locked<T: Send> {
    /// Receive wakers
    recv: Wake,
    /// Send wakers
    send: Wake,
    /// Data in transit
    data: Option<T>,
}

impl<T: Send> Default for Locked<T> {
    #[inline]
    fn default() -> Self {
        let data = None;
        let send = Wake::default();
        let recv = Wake::default();

        Self { data, send, recv }
    }
}

#[derive(Default)]
struct Shared<T: Send> {
    spin: spin::Spin<Locked<T>>,
}

/// A `Channel` notifies when another `Channel` sends a message.
///
/// Implemented as a multi-producer/multi-consumer queue of size 1.
///
/// Enable the **`futures-core`** feature for `Channel` to implement
/// [`Stream`](futures_core::Stream) (generic `T` must be `Option<Item>`).
///
/// Enable the **`pasts`** feature for `Channel` to implement
/// [`Notifier`](pasts::Notifier).
pub struct Channel<T: Send>(Arc<Shared<T>>);

impl<T: Send> core::fmt::Debug for Channel<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Channel")
            .field("strong_count", &Arc::strong_count(&self.0))
            .finish()
    }
}

impl<T: Send> Clone for Channel<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Send> Default for Channel<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send> Channel<T> {
    /// Create a new channel.
    #[inline]
    pub fn new() -> Self {
        let spin = spin::Spin::default();

        Self(Arc::new(Shared { spin }))
    }

    /// Send a message on this channel.
    #[inline(always)]
    pub fn send(&self, message: T) -> impl Future<Output = ()> + Send {
        Message((*self).clone(), Cell::new(Some(message)))
    }

    /// Receive a message from this channel.
    #[inline(always)]
    pub fn recv(&self) -> impl Future<Output = T> + Send + Sync + Unpin + '_ {
        self
    }

    /// Create a new corresponding [`Weak`] channel.
    #[inline]
    pub fn downgrade(&self) -> Weak<T> {
        Weak(Arc::downgrade(&self.0))
    }

    #[inline(always)]
    fn poll_internal(&self, cx: &mut Context<'_>) -> Poll<T> {
        let waker = cx.waker();
        let uid = Arc::as_ptr(&self.0) as usize;
        self.0.spin.with(|shared| {
            if let Some(output) = shared.data.take() {
                shared.send.wake();
                Ready(output)
            } else {
                shared.recv.register(uid, waker.clone());
                Pending
            }
        })
    }
}

impl<T: Send> Future for Channel<T> {
    type Output = T;

    #[inline(always)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_internal(cx)
    }
}

impl<T: Send> Future for &Channel<T> {
    type Output = T;

    #[inline(always)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_internal(cx)
    }
}

#[cfg(feature = "pasts")]
impl<T: Send> pasts::Notifier for Channel<T> {
    type Event = T;

    #[inline(always)]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.poll_internal(cx)
    }
}

#[cfg(feature = "pasts")]
impl<T: Send> pasts::Notifier for &Channel<T> {
    type Event = T;

    #[inline(always)]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.poll_internal(cx)
    }
}

#[cfg(feature = "futures-core")]
impl<T: Send> futures_core::Stream for Channel<Option<T>> {
    type Item = T;

    #[inline(always)]
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.poll_internal(cx)
    }
}

#[cfg(feature = "futures-core")]
impl<T: Send> futures_core::Stream for &Channel<Option<T>> {
    type Item = T;

    #[inline(always)]
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.poll_internal(cx)
    }
}

/// A weak refrence to a [`Channel`].
pub struct Weak<T: Send>(sync::Weak<Shared<T>>);

impl<T: Send> core::fmt::Debug for Weak<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Weak")
            .field("strong_count", &sync::Weak::strong_count(&self.0))
            .finish()
    }
}

impl<T: Send> Default for Weak<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send> Weak<T> {
    /// Calling `upgrade()` will always return `None`.
    #[inline]
    pub fn new() -> Self {
        Self(sync::Weak::new())
    }

    /// Attempt to upgrade the Weak channel to a [`Channel`].
    #[inline]
    pub fn upgrade(&self) -> Option<Channel<T>> {
        Some(Channel(self.0.upgrade()?))
    }
}

/// A message in the process of being sent over a [`Channel`].
struct Message<T: Send>(Channel<T>, Cell<Option<T>>);

#[allow(unsafe_code)]
impl<T: Send> Message<T> {
    #[inline(always)]
    fn pin_get(self: Pin<&Self>) -> Pin<&Cell<Option<T>>> {
        // This is okay because `1` is pinned when `self` is.
        unsafe { self.map_unchecked(|s| &s.1) }
    }
}

impl<T: Send> Future for Message<T> {
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = Pin::new(&self).get_ref();
        let waker = cx.waker();
        let uid = Arc::as_ptr(&this.0 .0) as usize;
        this.0 .0.spin.with(|shared| {
            if shared.data.is_none() {
                shared.data = this.as_ref().pin_get().take();
                shared.recv.wake();
                Ready(())
            } else {
                shared.send.register(uid, waker.clone());
                Pending
            }
        })
    }
}
