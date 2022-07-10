//! A simple and fast two-way async channel.
//!
//! The idea is based on a concept of calling a function on a different task.
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
//!     println!("Dropping worker…");
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

mod asym;

pub use asym::{Channel, Receiver, Sender};

/// Handle to a worker - command consumer (spsc-rendezvous channel)
#[derive(Debug)]
pub struct Worker<T: Send>(Sender<Option<T>>);

impl<T: Send> Worker<T> {
    /// Start up a worker (similar to the actor concept).
    #[inline]
    pub fn new(cb: impl FnOnce(Tasker<T>)) -> Self {
        let (sender, receiver) = Channel::pair();

        // Launch worker
        cb(Tasker(core::cell::Cell::new(Some(receiver))));

        // Return worker handle
        Self(sender)
    }

    /// Send an command to the worker.
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

impl<T: Send> Drop for Worker<T> {
    #[inline]
    fn drop(&mut self) {
        panic!("Worker dropped without stopping");
    }
}

/// Handle to a tasker - command producer (spsc-rendezvous channel)
pub struct Tasker<T: Send>(core::cell::Cell<Option<Receiver<Option<T>>>>);

impl<T: Send + core::fmt::Debug> core::fmt::Debug for Tasker<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let tmp = self.0.take();
        f.debug_struct("Foo").field("0", &tmp).finish()?;
        self.0.set(tmp);
        Ok(())
    }
}

impl<T: Send> Tasker<T> {
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

impl<T: Send> Drop for Tasker<T> {
    #[inline]
    fn drop(&mut self) {
        if self.0.take().is_some() {
            panic!("Tasker dropped before Worker");
        }
    }
}
