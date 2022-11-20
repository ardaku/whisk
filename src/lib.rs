//! #### Simple and fast async channels
//! Lightweight async channel that can be used to implement futures, streams,
//! notifiers, and actors.
//!
//! Whisk defines a simple [`Channel`] type rather than splitting into sender /
//! receiver pairs.  A [`Channel`] can both send and receive.
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
//! async fn worker_main(commands: Channel<Option<Cmd>>) {
//!     while let Some(command) = commands.recv().await {
//!         println!("Worker receiving command");
//!         match command {
//!             Cmd::Add(a, b, s) => s.send(a + b).await,
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
//!     let worker_task = worker_main(channel.clone());
//!     let worker_thread =
//!         std::thread::spawn(|| pasts::Executor::default().spawn(worker_task));
//!
//!     // Do an addition
//!     println!("Sending command…");
//!     let oneshot = Channel::new();
//!     channel.send(Some(Cmd::Add(43, 400, oneshot.clone()))).await;
//!     println!("Receiving response…");
//!     let response = oneshot.recv().await;
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
//!     pasts::Executor::default().spawn(tasker_main())
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

#[allow(unsafe_code)]
mod mutex;
#[allow(unsafe_code)]
mod wake_list;

use alloc::sync::Arc;
use core::{
    cell::Cell,
    future::{self, Future},
    pin::Pin,
    task::{
        Context,
        Poll::{self, Pending, Ready},
    },
};

use self::wake_list::{WakeHandle, WakeList};

/// A `Queue` can send messages to itself, and can be shared between threads
/// and tasks.
///
/// Implemented as a multi-producer/multi-consumer queue of size 1.
///
/// Enable the **`futures-core`** feature for `&Queue` to implement
/// [`Stream`](futures_core::Stream) (generic `T` must be `Option<Item>`).
///
/// Enable the **`pasts`** feature for `&Queue` to implement
/// [`Notifier`](pasts::Notifier).
pub struct Queue<T = (), U: ?Sized = ()> {
    /// Receive wakers
    recv: WakeList,
    /// Send wakers
    send: WakeList,
    /// Data in transit
    data: mutex::Mutex<Option<T>>,
    /// User data
    user: U,
}

impl<T, U: ?Sized> core::fmt::Debug for Queue<T, U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Queue").finish_non_exhaustive()
    }
}

impl<T, U: ?Sized> core::ops::Deref for Queue<T, U> {
    type Target = U;

    fn deref(&self) -> &Self::Target {
        &self.user
    }
}

impl<T, U: ?Sized + Default> Default for Queue<T, U> {
    fn default() -> Self {
        Self::with(U::default())
    }
}

impl<T> Queue<T> {
    /// Create a new channel.
    #[inline]
    pub const fn new() -> Self {
        Self::with(())
    }
}

impl<T, U> Queue<T, U> {
    /// Create a new channel with associated data.
    #[inline]
    pub const fn with(user_data: U) -> Self {
        Self {
            data: mutex::Mutex::new(None),
            send: wake_list::WakeList::new(),
            recv: wake_list::WakeList::new(),
            user: user_data,
        }
    }
}

impl<T, U: ?Sized> Queue<T, U> {
    /// Send a message on this channel.
    #[inline(always)]
    pub async fn send(&self, message: T) {
        Message(self, Cell::new(Some(message)), WakeHandle::new()).await
    }

    // Internal asynchronous receive implementation
    fn poll_internal(
        &self,
        cx: &mut Context<'_>,
        wake: &mut WakeHandle,
    ) -> Poll<T> {
        let waker = cx.waker();
        let poll = self.data.try_with(|inner| {
            if let Some(shared) = inner {
                if let Some(output) = shared.take() {
                    Ready(output)
                } else {
                    // Nothing is available yet, register waker for when it is
                    // Registration happens while owned to avoid data race
                    wake.register(&self.recv, waker.clone());
                    Pending
                }
            } else {
                // Data is contended, register and try again
                wake.register(&self.recv, waker.clone());
                self.data.try_with(|inner| {
                    if let Some(shared) = inner {
                        if let Some(output) = shared.take() {
                            // Unregister
                            *wake = WakeHandle::new();
                            return Ready(output);
                        }
                    }
                    Pending
                })
            }
        });

        // Any waking happens after the data is released
        poll.map(|x| {
            // Now that space is available, possibly wake a sender
            self.send.wake_one();
            x
        })
    }
}

/// A message in the process of being sent over a [`Queue`].
struct Message<'a, T, U: ?Sized>(&'a Queue<T, U>, Cell<Option<T>>, WakeHandle);

#[allow(unsafe_code)]
impl<T, U: ?Sized> Message<'_, T, U> {
    #[inline(always)]
    fn pin_get(self: Pin<&Self>) -> Pin<&Cell<Option<T>>> {
        // This is okay because `1` is pinned when `self` is.
        unsafe { self.map_unchecked(|s| &s.1) }
    }

    #[inline(always)]
    fn pin_get_wh(self: Pin<&mut Self>) -> Pin<&mut WakeHandle> {
        // This is okay because `1` is pinned when `self` is.
        unsafe { self.map_unchecked_mut(|s| &mut s.2) }
    }
}

impl<T, U: ?Sized> Future for Message<'_, T, U> {
    type Output = ();

    #[inline]
    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let waker = cx.waker();
        let poll = self.0.data.try_with(|inner| {
            if let Some(shared) = inner {
                if shared.is_none() {
                    // Buffer has space, write to it
                    *shared = self.as_ref().pin_get().take();
                    Ready(())
                } else {
                    // Registering while locked to avoid data race
                    let mut wh = WakeHandle::new();
                    core::mem::swap(
                        &mut wh,
                        self.as_mut().pin_get_wh().get_mut(),
                    );
                    wh.register(&self.0.send, waker.clone());
                    core::mem::swap(
                        &mut wh,
                        self.as_mut().pin_get_wh().get_mut(),
                    );
                    Pending
                }
            } else {
                // Data is contended, register and try again
                let mut wh = WakeHandle::new();
                core::mem::swap(&mut wh, self.as_mut().pin_get_wh().get_mut());
                wh.register(&self.0.send, waker.clone());
                core::mem::swap(&mut wh, self.as_mut().pin_get_wh().get_mut());
                self.0.data.try_with(|inner| {
                    if let Some(shared) = inner {
                        if shared.is_none() {
                            // Buffer has space, write to it
                            *shared = self.as_ref().pin_get().take();
                            // Unregister
                            let mut wh = WakeHandle::new();
                            core::mem::swap(
                                &mut wh,
                                self.as_mut().pin_get_wh().get_mut(),
                            );
                            return Ready(());
                        }
                    }

                    Pending
                })
            }
        });

        // Any waking happens after the data is released
        poll.map(|x| {
            // Now that data has been written, possibly wake a receiver
            self.0.recv.wake_one();
            x
        })
    }
}

/// An MPMC channel with both send and receive capabilities
pub struct Channel<T = (), U: ?Sized = ()>(Arc<Queue<T, U>>, WakeHandle);

impl<T> Channel<T> {
    /// Create a new channel.
    #[inline(always)]
    pub fn new() -> Self {
        Self(Arc::new(Queue::new()), WakeHandle::new())
    }
}

impl<T, U> Channel<T, U> {
    /// Create a new channel with associated data.
    #[inline(always)]
    pub fn with(user_data: U) -> Self {
        Self::from_inner(Arc::new(Queue::with(user_data)))
    }
}

impl<T, U: ?Sized> Channel<T, U> {
    /// Send a message on this channel.
    #[inline(always)]
    pub async fn send(&self, message: T) {
        self.0.send(message).await
    }

    /// Receive a message from this channel.
    #[inline(always)]
    pub async fn recv(&self) -> T {
        let mut wh = WakeHandle::new();
        future::poll_fn(|cx| self.0.poll_internal(cx, &mut wh)).await
    }

    /// Get the internal [`Arc`] out from the channel.
    #[inline(always)]
    pub fn into_inner(self) -> Arc<Queue<T, U>> {
        self.0
    }

    /// Create a new channel from a [`Queue`] wrapped in an [`Arc`].
    #[inline(always)]
    pub fn from_inner(inner: Arc<Queue<T, U>>) -> Self {
        Self(inner, WakeHandle::new())
    }
}

impl<T, U: ?Sized> Clone for Channel<T, U> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0), WakeHandle::new())
    }
}

impl<T, U: ?Sized> core::fmt::Debug for Channel<T, U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Channel").finish_non_exhaustive()
    }
}

impl<T, U: ?Sized + Default> Default for Channel<T, U> {
    fn default() -> Self {
        Self::with(U::default())
    }
}

impl<T, U: ?Sized> core::ops::Deref for Channel<T, U> {
    type Target = U;

    fn deref(&self) -> &Self::Target {
        &self.0.user
    }
}

impl<T, U: ?Sized> Future for Channel<T, U> {
    type Output = T;

    #[inline(always)]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let this = self.get_mut();
        this.0.poll_internal(cx, &mut this.1)
    }
}

#[cfg(feature = "pasts")]
impl<T, U: ?Sized> pasts::Notifier for Channel<T, U> {
    type Event = T;

    #[inline(always)]
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let this = self.get_mut();
        this.0.poll_internal(cx, &mut this.1)
    }
}

#[cfg(feature = "futures-core")]
impl<T, U: ?Sized> futures_core::Stream for Channel<Option<T>, U> {
    type Item = T;

    #[inline(always)]
    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<T>> {
        let this = self.get_mut();
        this.0.poll_internal(cx, &mut this.1)
    }
}
