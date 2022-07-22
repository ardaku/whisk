//! #### Simple and fast async channels
//! Simple and fast async channels that can be used to implement futures,
//! streams, notifiers, and actors.
//!
//! # Optional Features
//! The `std` feature is enabled by default, disable it to use on **no_std**.
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

extern crate alloc;

use alloc::{sync::Arc, vec::Vec};
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

#[derive(Debug, Default)]
struct Mutex {
    flag: AtomicBool,
}

impl Mutex {
    #[inline(always)]
    fn with<O>(&self, then: impl FnOnce() -> O) -> O {
        while self
            .flag
            .compare_exchange_weak(false, true, Relaxed, Relaxed)
            .is_err()
        {}
        atomic::fence(Acquire);
        let output = then();
        self.flag.store(false, Release);
        output
    }
}

#[derive(Debug)]
struct Locked<T: Send> {
    /// Data in transit
    data: Option<T>,
    /// Wakers
    wake: Vec<Waker>,
}

impl<T: Send> Default for Locked<T> {
    #[inline]
    fn default() -> Self {
        let data = None;
        let wake = Vec::new();

        Self { data, wake }
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
unsafe impl<T: Send + Unpin> Send for Channel<T> {}

/// A `Channel` notifies when another `Channel` sends a message.
///
/// Implemented as a multi-producer/multi-consumer queue of size 1
#[derive(Debug)]
pub struct Channel<T: Send + Unpin>(Arc<Shared<T>>);

impl<T: Send + Unpin> Clone for Channel<T> {
    #[inline]
    fn clone(&self) -> Self {
        Channel(Arc::clone(&self.0))
    }
}

impl<T: Send + Unpin> Default for Channel<T> {
    #[inline]
    fn default() -> Self {
        Self(Arc::new(Shared::default()))
    }
}

impl<T: Send + Unpin> Channel<T> {
    /// Create a new channel.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Send a message on this channel.
    #[inline(always)]
    pub fn send(&self, message: T) -> Message<T> {
        Message((*self).clone(), Some(message))
    }

    /// Receive a message from this channel.
    #[inline(always)]
    pub async fn recv(&self) -> T {
        self.clone().await
    }
}

impl<T: Send + Unpin> Future for Channel<T> {
    type Output = T;

    #[inline]
    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = &mut self.as_mut().0;
        let waker = cx.waker();
        this.mutex.with(|| {
            if let Some(output) = unsafe { (*this.data.get()).data.take() } {
                for waker in unsafe { (*this.data.get()).wake.drain(..) } {
                    waker.wake();
                }
                Ready(output)
            } else {
                unsafe { (*this.data.get()).wake.push(waker.clone()) };
                Pending
            }
        })
    }
}

/// A message in the process of being sent over a [`Channel`].
#[derive(Debug)]
pub struct Message<T: Send + Unpin>(Channel<T>, Option<T>);

impl<T: Send + Unpin> Future for Message<T> {
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let waker = cx.waker();
        this.0 .0.mutex.with(|| {
            let lock = unsafe { &mut (*this.0 .0.data.get()) };
            if lock.data.is_none() {
                lock.data = this.1.take();
                for waker in lock.wake.drain(..) {
                    waker.wake();
                }
                Ready(())
            } else {
                lock.wake.push(waker.clone());
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
