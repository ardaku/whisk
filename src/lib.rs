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
//!     /// Tell messenger to quit
//!     Stop,
//! }
//!
//! async fn worker(tasker: Tasker<Cmd>) {
//!     loop {
//!         println!("Worker receiving command");
//!         match tasker.recv_next().await {
//!             Cmd::Add(a, b, s) => s.send(a + b),
//!             Cmd::Stop => break,
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
//!     worker.send(Cmd::Add(43, 400, send));
//!     println!("Receiving response…");
//!     let response = recv.recv().await;
//!     assert_eq!(response, 443);
//!
//!     // Tell worker to stop
//!     println!("Stopping worker…");
//!     worker.send(Cmd::Stop);
//!
//!     // Close channel
//!     println!("Closing channel…");
//!     drop(worker);
//!     println!("Closed channel…");
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

/// Handle to a worker
#[derive(Debug)]
pub struct Worker<T: Send>(Sender<T>);

impl<T: Send> Worker<T> {
    /// Start up a worker (similar to the actor concept).
    pub fn new(cb: impl FnOnce(Tasker<T>)) -> Self {
        let (sender, receiver) = Channel::pair();

        // Launch worker
        cb(Tasker(receiver));

        // Return worker handle
        Self(sender)
    }

    /// Send an command to the worker.
    pub fn send(&self, cmd: T) {
        self.0.send_and_reuse(cmd);
    }
}

impl<T: Send> Drop for Worker<T> {
    fn drop(&mut self) {
        self.0.unuse();
    }
}

/// Handle to a tasker
#[derive(Debug)]
pub struct Tasker<T: Send>(Receiver<T>);

impl<T: Send> Tasker<T> {
    /// Get the next command from the tasker
    pub async fn recv_next(&self) -> T {
        self.0.recv_and_reuse().await
    }
}

impl<T: Send> Drop for Tasker<T> {
    fn drop(&mut self) {
        self.0.unuse();
    }
}
