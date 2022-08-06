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

use alloc::{
    sync::{self, Arc},
    vec::Vec,
};
use core::{
    cell::UnsafeCell,
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
mod wake {
    use super::*;

    /// Type for waking on send or receive
    #[derive(Debug, Default)]
    pub(super) struct Wake {
        /// Channel unique identifier (the arc pointer casted to usize)
        chan: usize,
        /// Channel waker
        wake: Option<Waker>,
        /// Heap wakers
        list: Vec<(usize, Waker)>,
    }

    impl Wake {
        /// Register a waker for a channel
        #[inline(always)]
        pub(super) fn register(&mut self, chan: usize, waker: Waker) {
            if self.list.is_empty() {
                if let Some(wake) = self.wake.take() {
                    if self.chan == chan {
                        (self.chan, self.wake) = (chan, Some(waker));
                    } else {
                        self.list.extend([(self.chan, wake), (chan, waker)]);
                    }
                } else {
                    (self.chan, self.wake) = (chan, Some(waker));
                }
            } else {
                if let Some(wake) = self.list.iter_mut().find(|w| w.0 == chan) {
                    wake.1 = waker;
                } else {
                    self.list.push((chan, waker));
                }
            }
        }

        /// Wake all channels and de-register all wakers
        #[inline(always)]
        pub(super) fn wake(&mut self) {
            if let Some(waker) = self.wake.take() {
                waker.wake();
                return;
            }
            for waker in self.list.drain(..) {
                waker.1.wake();
            }
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
struct Locked<T: Send> {
    /// Receive wakers
    recv: wake::Wake,
    /// Send wakers
    send: wake::Wake,
    /// Data in transit
    data: Option<T>,
}

impl<T: Send> Default for Locked<T> {
    #[inline]
    fn default() -> Self {
        let data = None;
        let send = wake::Wake::default();
        let recv = wake::Wake::default();

        Self { data, send, recv }
    }
}

#[derive(Debug, Default)]
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
#[derive(Debug)]
pub struct Channel<T: Send + Unpin>(Arc<Shared<T>>);

impl<T: Send + Unpin> Clone for Channel<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Send + Unpin> Default for Channel<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + Unpin> Channel<T> {
    /// Create a new channel.
    #[inline]
    pub fn new() -> Self {
        let spin = spin::Spin::default();

        Self(Arc::new(Shared { spin }))
    }

    /// Send a message on this channel.
    #[inline(always)]
    pub fn send(&self, message: T) -> impl Future<Output = ()> + Send + Unpin {
        Message((*self).clone(), Some(message))
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
    pub fn downgrade(&self) -> Weak<T> {
        Weak(Arc::downgrade(&self.0))
    }
}

impl<T: Send + Unpin> Future for Channel<T> {
    type Output = T;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let waker = cx.waker();
        let uid = Arc::as_ptr(&this.0) as usize;
        this.0.spin.with(|shared| {
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

#[cfg(feature = "pasts")]
impl<T: Send + Unpin> pasts::Notifier for Channel<T> {
    type Event = T;

    #[inline(always)]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.poll(cx)
    }
}

#[cfg(feature = "futures-core")]
impl<T: Send + Unpin> futures_core::Stream for Channel<Option<T>> {
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
pub struct Weak<T: Send + Unpin>(sync::Weak<Shared<T>>);

impl<T: Send + Unpin> Weak<T> {
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
#[derive(Debug)]
struct Message<T: Send + Unpin>(Channel<T>, Option<T>);

impl<T: Send + Unpin> Future for Message<T> {
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let waker = cx.waker();
        let uid = Arc::as_ptr(&this.0 .0) as usize;
        this.0 .0.spin.with(|shared| {
            if shared.data.is_none() {
                shared.data = this.1.take();
                shared.recv.wake();
                Ready(())
            } else {
                shared.send.register(uid, waker.clone());
                Pending
            }
        })
    }
}

impl<T: Send + Unpin> Drop for Message<T> {
    fn drop(&mut self) {
        if self.1.is_some() {
            panic!("Message dropped without sending");
        }
    }
}
