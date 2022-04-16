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
//!                 command.close(messenger);
//!                 return
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
//!     for message in commander {
//!         let responder = match message.get() {
//!             Msg::Ready => {
//!                 println!("Commander received ready, sending add command…");
//!                 message.respond(Cmd::Add(43, 400))
//!             }
//!             Msg::Response(value) => {
//!                 assert_eq!(*value, 443);
//!                 println!("Commander received response, sending exit command…");
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
    mem::{self, ManuallyDrop},
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
                    if (*this.0).leased {
                        panic!("Didn't respond before next .await");
                    }

                    let message = if let Some(ref mut data) = (*this.0).data {
                        Some(ManuallyDrop::new(ManuallyDrop::take(
                            &mut data.message,
                        )))
                    } else {
                        None
                    };

                    if let Some(message) = message {
                        let waker = (*this.0).waker.take().unwrap();
                        let message = Message {
                            waker: ManuallyDrop::new(waker),
                            inner: message,
                            internal: this.0,
                        };

                        (*this.0).waker = Some(cx.waker().clone());
                        (*this.0).leased = true;
                        (*this.0).message = Some(message);
                    }

                    Poll::Ready(())
                } else {
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
                    if (*this.0).leased {
                        panic!("Didn't respond before next .await");
                    }

                    if (*this.0).first {
                        let waker = (*this.0).waker.take().unwrap();

                        (*this.0).first = false;
                        (*this.0).owner.0.store(COMMANDER, Ordering::Release);
                        (*this.0).waker = Some(cx.waker().clone());

                        waker.wake();

                        return Poll::Pending;
                    }

                    let command = if let Some(ref mut data) = (*this.0).data {
                        Some(ManuallyDrop::new(ManuallyDrop::take(
                            &mut data.command,
                        )))
                    } else {
                        None
                    };

                    if let Some(command) = command {
                        let waker = (*this.0).waker.take().unwrap();
                        let command = Command {
                            waker: ManuallyDrop::new(waker),
                            inner: command,
                            internal: this.0,
                        };

                        (*this.0).waker = Some(cx.waker().clone());
                        (*this.0).leased = true;
                        (*this.0).command = Some(command);
                    }

                    Poll::Ready(())
                } else {
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
    /// Who owns the data
    owner: Owner,
    /// True if it is unsound to free this structure
    leased: bool,
    /// True if ready message has not been sent yet
    first: bool,
    /// Waker of other task
    waker: Option<Waker>,
    /// Union, because discriminant is `owner` field.
    ///
    /// None on disconnect
    data: Option<Data<Cmd, Msg>>,

    /// Owned by messsenger, next command.
    command: Option<Command<Cmd, Msg>>,
    /// Owned by commander, next message.
    message: Option<Message<Cmd, Msg>>,
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

impl<Cmd: Send, Msg: Send> Iterator for Commander<Cmd, Msg> {
    type Item = Message<Cmd, Msg>;

    fn size_hint(&self) -> (usize, Option<usize>) {
        (usize::MAX, None)
    }

    fn count(self) -> usize {
        unimplemented!()
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next()
    }

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

            if (*self.0).leased {
                // Panic so that `Command` can't use-after-free.
                //
                // Reaching this should always be considered a bug in the
                // calling code
                panic!("Cannot drop `Commander` before `Command`");
            } else if (*self.0).data.is_some() {
                // Let messenger know commander is gone
                (*self.0).data = None;
                (*self.0).owner.0.store(MESSENGER, Ordering::Release);
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

impl<Cmd: Send, Msg: Send> Iterator for Messenger<Cmd, Msg> {
    type Item = Command<Cmd, Msg>;

    fn size_hint(&self) -> (usize, Option<usize>) {
        (usize::MAX, None)
    }

    fn count(self) -> usize {
        unimplemented!()
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next()
    }

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

            if (*self.0).leased {
                // Panic so that `Message` can't use-after-free.
                //
                // Reaching this should always be considered a bug in the
                // calling code
                panic!("Cannot drop `Messenger` before `Message`");
            } else if (*self.0).data.is_some() {
                // Let commander know messenger is gone
                (*self.0).data = None;
                (*self.0).owner.0.store(COMMANDER, Ordering::Release);
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
            owner: Owner(AtomicBool::new(MESSENGER)),
            leased: false,
            first: true,
            waker: Some(waker),
            data: Some(Data {
                message: ManuallyDrop::new(self.0.take().unwrap()),
            }),
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
pub struct Command<Cmd: Send, Msg: Send> {
    waker: ManuallyDrop<Waker>,
    inner: ManuallyDrop<Cmd>,
    internal: *mut Internal<Cmd, Msg>,
}

unsafe impl<Cmd: Send, Msg: Send> Send for Command<Cmd, Msg> {}

impl<Cmd: Send, Msg: Send> Command<Cmd, Msg> {
    /// Get the received command
    #[inline(always)]
    pub fn get(&self) -> &Cmd {
        &self.inner
    }

    /// Respond to the receieved command
    #[inline(always)]
    pub fn respond(self, message: Msg) -> impl Future<Output = ()> + Send {
        let mut command = self;

        unsafe {
            // Set response message
            if let Some(ref mut data) = (*command.internal).data {
                data.message = ManuallyDrop::new(message);
            }

            // Release control to commander
            (*command.internal).leased = false;
            (*command.internal)
                .owner
                .0
                .store(COMMANDER, Ordering::Release);
            ManuallyDrop::take(&mut command.waker).wake();

            // Manual drop of inner
            drop(ManuallyDrop::take(&mut command.inner));
        }

        // Forget self
        let internal = command.internal;
        mem::forget(command);

        // Create messenger future
        seal::MessengerFuture(internal)
    }

    /// Respond by closing the channel.
    #[inline(always)]
    pub fn close(self, messenger: Messenger<Cmd, Msg>) {
        let mut command = self;

        unsafe {
            // Release control to commander
            (*command.internal).leased = false;
            // Drop messenger,
            drop(messenger);
            // Notify commander
            ManuallyDrop::take(&mut command.waker).wake();
            // Manual drop of inner
            drop(ManuallyDrop::take(&mut command.inner));
        }

        // Forget self
        mem::forget(command);
    }
}

/// Communication from the [`Messenger`].
#[must_use]
#[derive(Debug)]
pub struct Message<Cmd: Send, Msg: Send> {
    waker: ManuallyDrop<Waker>,
    inner: ManuallyDrop<Msg>,
    internal: *mut Internal<Cmd, Msg>,
}

unsafe impl<Cmd: Send, Msg: Send> Send for Message<Cmd, Msg> {}

impl<Cmd: Send, Msg: Send> Message<Cmd, Msg> {
    /// Get the received message
    #[inline(always)]
    pub fn get(&self) -> &Msg {
        &self.inner
    }

    /// Respond to the receieved message
    #[inline(always)]
    pub fn respond(self, command: Cmd) -> impl Future<Output = ()> + Send {
        let mut message = self;

        unsafe {
            // Set response command
            if let Some(ref mut data) = (*message.internal).data {
                data.command = ManuallyDrop::new(command);
            }

            // Release control to messenger
            (*message.internal).leased = false;
            (*message.internal)
                .owner
                .0
                .store(MESSENGER, Ordering::Release);
            ManuallyDrop::take(&mut message.waker).wake();

            // Manual drop of inner
            drop(ManuallyDrop::take(&mut message.inner));
        }

        // Forget self
        let internal = message.internal;
        mem::forget(message);

        // Create commander future
        seal::CommanderFuture(internal)
    }

    /// Respond by closing the channel.
    #[inline(always)]
    pub fn close(self, commander: Commander<Cmd, Msg>) {
        let mut message = self;

        unsafe {
            // Release control to messenger
            (*message.internal).leased = false;
            // Drop commander
            drop(commander);
            // Notify messenger
            ManuallyDrop::take(&mut message.waker).wake();
            // Manual drop of inner
            drop(ManuallyDrop::take(&mut message.inner));
        }

        // Forget self
        mem::forget(message);
    }
}
