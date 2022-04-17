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
//! use whisk::Messenger;
//!
//! enum Msg {
//!     /// Messenger has finished initialization
//!     Ready,
//!     /// Result of addition
//!     Response(u32),
//! }
//!
//! enum Cmd {
//!     /// Tell messenger to add
//!     Add(u32, u32),
//!     /// Tell messenger to quit
//!     Exit,
//! }
//!
//! async fn messenger_task(mut messenger: Messenger<Cmd, Msg>) {
//!     // Send ready and receive command from commander
//!     println!("Messenger sending ready");
//!     messenger.start().await;
//!
//!     for command in &mut messenger {
//!         let responder = match command.get() {
//!             Cmd::Add(a, b) => {
//!                 println!("Messenger received add, sending response");
//!                 let result = *a + *b;
//!                 command.respond(Msg::Response(result))
//!             }
//!             Cmd::Exit => {
//!                 println!("Messenger received exit, shutting down…");
//!                 return;
//!             }
//!         };
//!         responder.await
//!     }
//!
//!     unreachable!()
//! }
//!
//! async fn commander_task() {
//!     let (mut commander, messenger) = whisk::channel(Msg::Ready).await;
//!
//!     // Start messenger task on another thread
//!     let messenger = messenger_task(messenger);
//!     let messenger = std::thread::spawn(|| pasts::block_on(messenger));
//!
//!     // Wait for Ready message, and respond with Exit command
//!     println!("Commander waiting ready message…");
//!     commander.start().await;
//!     for message in &mut commander {
//!         let responder = match message.get() {
//!             Msg::Ready => {
//!                 println!("Commander received ready, sending add command…");
//!                 message.respond(Cmd::Add(43, 400))
//!             }
//!             Msg::Response(value) => {
//!                 assert_eq!(*value, 443);
//!                 println!("Commander received response, commanding exit…");
//!                 message.respond(Cmd::Exit)
//!             }
//!         };
//!         responder.await
//!     }
//!
//!     println!("Commander disconnected");
//!     messenger.join().unwrap();
//!     println!("Messenger thread joined");
//! }
//!
//! // Call into executor of your choice
//! fn main() {
//!     pasts::block_on(commander_task())
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
    mem::ManuallyDrop,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, Waker},
};
#[cfg(feature = "std")]
use std::thread;

// Sealed futures
mod seal {
    use super::*;

    #[derive(Debug)]
    pub(super) struct CommanderFuture<Cmd: Send, Msg: Send>(
        pub(super) *mut Internal<Cmd, Msg>,
    );

    unsafe impl<Cmd: Send, Msg: Send> Send for CommanderFuture<Cmd, Msg> {}

    #[derive(Debug)]
    pub(super) struct MessengerFuture<Cmd: Send, Msg: Send>(
        pub(super) *mut Internal<Cmd, Msg>,
    );

    unsafe impl<Cmd: Send, Msg: Send> Send for MessengerFuture<Cmd, Msg> {}

    impl<Cmd: Send, Msg: Send> Future for CommanderFuture<Cmd, Msg> {
        type Output = ();

        #[inline(always)]
        fn poll(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Self::Output> {
            let this = &mut self.get_mut();

            unsafe {
                if (*this.0).owner.0.load(Ordering::Acquire) == COMMANDER {
                    let message = if let Some(ref mut data) = (*this.0).data {
                        Some(ManuallyDrop::new(ManuallyDrop::take(
                            &mut data.message,
                        )))
                    } else {
                        None
                    };

                    (*this.0).message = message.map(|mut message| Message {
                        inner: ManuallyDrop::take(&mut message),
                        internal: this.0,
                        _phantom: PhantomData,
                    });
                    Poll::Ready(())
                } else {
                    Internal::set_commander_waker(this.0, cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }

    impl<Cmd: Send, Msg: Send> Future for MessengerFuture<Cmd, Msg> {
        type Output = ();

        #[inline(always)]
        fn poll(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Self::Output> {
            let this = &mut self.get_mut();

            unsafe {
                if (*this.0).owner.0.load(Ordering::Acquire) == MESSENGER {
                    if (*this.0).first {
                        (*this.0).first = false;
                        (*this.0).owner.0.store(COMMANDER, Ordering::Release);

                        Internal::set_messenger_waker(
                            this.0,
                            cx.waker().clone(),
                        );
                        Internal::commander_wake(this.0);

                        return Poll::Pending;
                    }

                    let command = if let Some(ref mut data) = (*this.0).data {
                        Some(ManuallyDrop::new(ManuallyDrop::take(
                            &mut data.command,
                        )))
                    } else {
                        None
                    };

                    (*this.0).command = command.map(|mut command| Command {
                        inner: ManuallyDrop::take(&mut command),
                        internal: this.0,
                        _phantom: PhantomData,
                    });
                    Poll::Ready(())
                } else {
                    Internal::set_messenger_waker(this.0, cx.waker().clone());
                    Poll::Pending
                }
            }
        }
    }
}

struct Owner(AtomicBool);

const COMMANDER: bool = false;
const MESSENGER: bool = true;

union Data<Cmd, Msg> {
    /// Command for child
    command: ManuallyDrop<Cmd>,
    /// Message for parent
    message: ManuallyDrop<Msg>,
}

struct Internal<Cmd: Send, Msg: Send> {
    /// True if ready message has not been sent yet
    first: bool,
    /// Union, because discriminant is `owner` field.
    ///
    /// None on disconnect
    data: Option<Data<Cmd, Msg>>,
    /// Who owns the data
    owner: Owner,

    /// Owned by messenger, next command.
    command: Option<Command<'static, Cmd, Msg>>,
    /// Shared by messenger, commander waker.
    commander_waker: Option<Waker>,
    commander_waker_locked: AtomicBool,

    /// Owned by commander, next message.
    message: Option<Message<'static, Cmd, Msg>>,
    /// Shared by commander, messenger waker.
    messenger_waker: Option<Waker>,
    messenger_waker_locked: AtomicBool,
}

impl<Cmd: Send, Msg: Send> Internal<Cmd, Msg> {
    #[inline(always)]
    unsafe fn acquire_lock(lock: &AtomicBool) {
        // Spinlock wait
        while lock
            .compare_exchange_weak(
                false,
                true,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_err()
        {
            #[cfg(feature = "std")]
            thread::yield_now()
        }
    }

    #[inline(always)]
    fn commander_wake(internal: *mut Self) {
        unsafe {
            // Acquire lock
            Self::acquire_lock(&(*internal).commander_waker_locked);
            // Take & wake
            if let Some(waker) = (*internal).commander_waker.take() {
                waker.wake();
            }
            // Release lock
            (*internal)
                .commander_waker_locked
                .store(false, Ordering::Release);
        }
    }

    #[inline(always)]
    fn messenger_wake(internal: *mut Self) {
        unsafe {
            // Acquire lock
            Self::acquire_lock(&(*internal).messenger_waker_locked);
            // Take & wake
            if let Some(waker) = (*internal).messenger_waker.take() {
                waker.wake();
            }
            // Release lock
            (*internal)
                .messenger_waker_locked
                .store(false, Ordering::Release);
        }
    }

    #[inline(always)]
    fn set_commander_waker(internal: *mut Self, waker: Waker) {
        unsafe {
            // Acquire lock
            Self::acquire_lock(&(*internal).commander_waker_locked);
            // Insert new waker
            (*internal).commander_waker = Some(waker);
            // Release lock
            (*internal)
                .commander_waker_locked
                .store(false, Ordering::Release);
        }
    }

    #[inline(always)]
    fn set_messenger_waker(internal: *mut Self, waker: Waker) {
        unsafe {
            // Acquire lock
            Self::acquire_lock(&(*internal).messenger_waker_locked);
            // Insert new waker
            (*internal).messenger_waker = Some(waker);
            // Release lock
            (*internal)
                .messenger_waker_locked
                .store(false, Ordering::Release);
        }
    }
}

/// A commander tells the messenger what to do.
#[derive(Debug)]
pub struct Commander<Cmd: Send, Msg: Send>(*mut Internal<Cmd, Msg>);

unsafe impl<Cmd: Send, Msg: Send> Send for Commander<Cmd, Msg> {}

impl<Cmd: Send, Msg: Send> Commander<Cmd, Msg> {
    /// Fetch the first message.  This should only be called once.
    pub fn start(&mut self) -> impl Future<Output = ()> + Send {
        seal::CommanderFuture(self.0)
    }
}

impl<'a, Cmd: Send, Msg: Send> Iterator for &'a mut Commander<Cmd, Msg> {
    type Item = Message<'a, Cmd, Msg>;

    fn next(&mut self) -> Option<Self::Item> {
        // Safe because the other end doesn't use this field
        unsafe { (*self.0).message.take() }
    }
}

impl<Cmd: Send, Msg: Send> Drop for Commander<Cmd, Msg> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            // Wait for message from messenger
            while (*self.0).owner.0.load(Ordering::Acquire) != COMMANDER {
                #[cfg(feature = "std")]
                thread::yield_now()
            }

            if (*self.0).data.is_some() {
                // Let messenger know commander is gone
                (*self.0).data = None;
                (*self.0).owner.0.store(MESSENGER, Ordering::Release);
                Internal::messenger_wake(self.0);
            } else {
                // Messenger is gone, so the commander has to clean up
                let _ = Box::from_raw(self.0);
            }
        }
    }
}

/// A messenger reports the results of tasks the commander assigned.
#[derive(Debug)]
pub struct Messenger<Cmd: Send, Msg: Send>(*mut Internal<Cmd, Msg>);

unsafe impl<Cmd: Send, Msg: Send> Send for Messenger<Cmd, Msg> {}

impl<Cmd: Send, Msg: Send> Messenger<Cmd, Msg> {
    /// Fetch the first command.  This should only be called once.
    pub fn start(&mut self) -> impl Future<Output = ()> + Send {
        seal::MessengerFuture(self.0)
    }
}

impl<'a, Cmd: Send, Msg: Send> Iterator for &'a mut Messenger<Cmd, Msg> {
    type Item = Command<'a, Cmd, Msg>;

    fn next(&mut self) -> Option<Self::Item> {
        // Safe because the other end doesn't use this field
        unsafe { (*self.0).command.take() }
    }
}

impl<Cmd: Send, Msg: Send> Drop for Messenger<Cmd, Msg> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            // Wait for command from commander
            while (*self.0).owner.0.load(Ordering::Acquire) != MESSENGER {
                #[cfg(feature = "std")]
                thread::yield_now()
            }

            if (*self.0).data.is_some() {
                // Let commander know messenger is gone
                (*self.0).data = None;
                (*self.0).owner.0.store(COMMANDER, Ordering::Release);
                Internal::commander_wake(self.0);
            } else {
                // Commander is gone, so the messenger has to clean up
                let _ = Box::from_raw(self.0);
            }
        }
    }
}

struct Channel<Cmd: Unpin, Msg: Unpin>(Option<Msg>, PhantomData<Cmd>);

impl<Cmd: Unpin + Send, Msg: Unpin + Send> Future for Channel<Cmd, Msg> {
    type Output = (Commander<Cmd, Msg>, Messenger<Cmd, Msg>);

    #[inline(always)]
    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let waker = cx.waker().clone();
        let internal = Internal {
            first: true,
            owner: Owner(AtomicBool::new(MESSENGER)),
            data: Some(Data {
                message: ManuallyDrop::new(self.0.take().unwrap()),
            }),
            commander_waker: Some(waker),
            commander_waker_locked: AtomicBool::new(false),
            messenger_waker: None,
            messenger_waker_locked: AtomicBool::new(false),
            command: None,
            message: None,
        };
        let internal = Box::leak(Box::new(internal));
        let output = (Commander(internal), Messenger(internal));

        Poll::Ready(output)
    }
}

/// Create an asynchronous 2-way channel between two tasks.
///
/// Should be called on commander task.  Usually, the first message from the
/// messenger task will be Ready for commands.
#[inline(always)]
pub fn channel<Cmd: Unpin + Send, Msg: Unpin + Send>(
    ready: Msg,
) -> impl Future<Output = (Commander<Cmd, Msg>, Messenger<Cmd, Msg>)> {
    Channel(Some(ready), PhantomData)
}

/// Communication from the [`Commander`].
#[must_use]
#[derive(Debug)]
pub struct Command<'a, Cmd: Send, Msg: Send> {
    inner: Cmd,
    internal: *mut Internal<Cmd, Msg>,
    _phantom: PhantomData<&'a mut ()>,
}

unsafe impl<Cmd: Send, Msg: Send> Send for Command<'_, Cmd, Msg> {}

impl<Cmd: Send, Msg: Send> Command<'_, Cmd, Msg> {
    /// Get the received command
    #[inline(always)]
    pub fn get(&self) -> &Cmd {
        &self.inner
    }

    /// Respond to the receieved command
    #[inline(always)]
    pub fn respond(self, message: Msg) -> impl Future<Output = ()> + Send {
        let internal = self.internal;

        unsafe {
            // Set response message
            if let Some(ref mut data) = (*internal).data {
                data.message = ManuallyDrop::new(message);
            }

            // Release control to commander
            (*internal).owner.0.store(COMMANDER, Ordering::Release);
            Internal::commander_wake(internal);
        }

        // Create messenger future
        seal::MessengerFuture(internal)
    }
}

/// Communication from the [`Messenger`].
#[must_use]
#[derive(Debug)]
pub struct Message<'a, Cmd: Send, Msg: Send> {
    inner: Msg,
    internal: *mut Internal<Cmd, Msg>,
    _phantom: PhantomData<&'a mut ()>,
}

unsafe impl<Cmd: Send, Msg: Send> Send for Message<'_, Cmd, Msg> {}

impl<Cmd: Send, Msg: Send> Message<'_, Cmd, Msg> {
    /// Get the received message
    #[inline(always)]
    pub fn get(&self) -> &Msg {
        &self.inner
    }

    /// Respond to the receieved message
    #[inline(always)]
    pub fn respond(self, command: Cmd) -> impl Future<Output = ()> + Send {
        let internal = self.internal;

        unsafe {
            // Set response command
            if let Some(ref mut data) = (*internal).data {
                data.command = ManuallyDrop::new(command);
            }

            // Release control to messenger
            (*internal).owner.0.store(MESSENGER, Ordering::Release);
            Internal::messenger_wake(internal);
        }

        // Create commander future
        seal::CommanderFuture(internal)
    }
}
