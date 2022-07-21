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
//! use whisk::{Channel, Sender, Tasker};
//! 
//! enum Cmd {
//!     /// Tell messenger to add
//!     Add(u32, u32, Sender<u32>),
//! }
//! 
//! async fn worker_main(mut tasker: Tasker<Cmd>) {
//!     while let Some(command) = (&mut tasker).await {
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
//!     let (worker, tasker) = Channel::new().spsc();
//!     let worker_thread = std::thread::spawn(move || {
//!         pasts::Executor::default()
//!             .spawn(Box::pin(async move { worker_main(tasker).await }))
//!     });
//! 
//!     // Do an addition
//!     println!("Sending command…");
//!     let (send, recv) = Channel::new().oneshot();
//!     let worker = worker.send(Cmd::Add(43, 400, send)).await;
//!     println!("Receiving response…");
//!     let response = recv.recv().await;
//!     assert_eq!(response, 443);
//! 
//!     // Tell worker to stop
//!     println!("Stopping worker…");
//!     worker.stop().await;
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
#[cfg(feature = "std")]
extern crate std;

// SpinLock implementation (used for transfer of wakers)
mod spin;/*
// Channel implementation
mod chan;
// Spsc implementation
mod spsc;*/
// Watch implementation
mod watch;

// pub use chan::{Channel, Receiver, Receiving, Sender, Sending};
// pub use spsc::{Message, Tasker, Worker};

pub use watch::{Channel, Message};
