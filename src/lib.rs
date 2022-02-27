//! A simple and fast two-way async channel.
//!
//! The idea is based on a concept of calling a function on a different task.
//!
//! # Getting Started
//!

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

struct Owner(AtomicBool);

const COMMANDER: bool = false;
const MESSENGER: bool = true;

union Data<Command, Message> {
    /// Command for child
    command: ManuallyDrop<Command>,
    /// Message for parent
    message: ManuallyDrop<Message>,
}

struct Internal<Command, Message> {
    /// Who owns the data
    owner: Owner,
    /// True if it is unsound to free this structure
    leased: bool,
    /// Waker of other task
    waker: Option<Waker>,
    /// Union, because discriminant is `owner` field.
    ///
    /// None on disconnect
    data: Option<Data<Command, Message>>,
}

/// A commander tells the messenger what to do.
#[derive(Debug)]
pub struct Commander<Command, Message>(*mut Internal<Command, Message>);

unsafe impl<Command: Send, Message: Send> Send for Commander<Command, Message> {}

impl<Cmd, Msg> Future for Commander<Cmd, Msg> {
    type Output = Option<Message<Cmd, Msg>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            if (*self.0).leased {
                panic!("Didn't respond before next .await");
            }

            if (*self.0).owner.0.load(Ordering::Acquire) == COMMANDER {
                let message = if let Some(ref mut data) = (*self.0).data {
                    Some(ManuallyDrop::new(ManuallyDrop::take(&mut data.message)))
                } else {
                    None
                };

                if let Some(message) = message {
                    let waker = (*self.0).waker.take().unwrap();
                    let message = Message {
                        waker: ManuallyDrop::new(waker),
                        inner: message,
                        internal: self.0,
                    };

                    (*self.0).waker = Some(cx.waker().clone());

                    Poll::Ready(Some(message))
                } else {
                    Poll::Ready(None)
                }
            } else {
                Poll::Pending
            }
        }
    }
}

impl<Command, Message> Drop for Commander<Command, Message> {
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
                // Let other messenger know commander is gone
                (*self.0).data = None;
                (*self.0).owner.0.store(MESSENGER, Ordering::Release);
                (*self.0).waker.take().unwrap().wake();
            } else {
                // Messenger is gone, so the commander has to clean up
                let _ = Box::from_raw(self.0);
            }
        }
    }
}

/// A messenger reports the results of tasks the commander assigned.
#[derive(Debug)]
pub struct Messenger<Command, Message>(*mut Internal<Command, Message>);

unsafe impl<Command: Send, Message: Send> Send for Messenger<Command, Message> {}

impl<Cmd, Msg> Future for Messenger<Cmd, Msg> {
    type Output = Option<Command<Cmd, Msg>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            if (*self.0).leased {
                panic!("Didn't respond before next .await");
            }

            if (*self.0).owner.0.load(Ordering::Acquire) == MESSENGER {
                let command = if let Some(ref mut data) = (*self.0).data {
                    Some(ManuallyDrop::new(ManuallyDrop::take(&mut data.command)))
                } else {
                    None
                };

                if let Some(command) = command {
                    let waker = (*self.0).waker.take().unwrap();
                    let command = Command {
                        waker: ManuallyDrop::new(waker),
                        inner: command,
                        internal: self.0,
                    };

                    (*self.0).waker = Some(cx.waker().clone());

                    Poll::Ready(Some(command))
                } else {
                    Poll::Ready(None)
                }
            } else {
                Poll::Pending
            }
        }
    }
}

impl<Command, Message> Drop for Messenger<Command, Message> {
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
                // Let other commander know messenger is gone
                (*self.0).data = None;
                (*self.0).owner.0.store(COMMANDER, Ordering::Release);
                (*self.0).waker.take().unwrap().wake();
            } else {
                // Commander is gone, so the messenger has to clean up
                let _ = Box::from_raw(self.0);
            }
        }
    }
}

struct Channel<Command: Unpin, Message: Unpin>(Option<Message>, PhantomData<Command>);

impl<Cmd, Msg> Future for Channel<Cmd, Msg>
where
    Cmd: Unpin,
    Msg: Unpin,
{
    type Output = (Commander<Cmd, Msg>, Messenger<Cmd, Msg>);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let waker = cx.waker().clone();

        let internal = Internal {
            owner: Owner(AtomicBool::new(MESSENGER)),
            leased: false,
            waker: Some(waker),
            data: Some(Data {
                message: ManuallyDrop::new(self.0.take().unwrap()),
            }),
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
pub fn channel<Command, Message>(
    ready: Message,
) -> impl Future<Output = (Commander<Command, Message>, Messenger<Command, Message>)>
where
    Command: Unpin,
    Message: Unpin,
{
    Channel(Some(ready), PhantomData)
}

/// Communication from the [`Commander`].
#[must_use]
#[derive(Debug)]
pub struct Command<Cmd, Msg> {
    waker: ManuallyDrop<Waker>,
    inner: ManuallyDrop<Cmd>,
    internal: *mut Internal<Cmd, Msg>,
}

impl<Cmd, Msg> Command<Cmd, Msg> {
    /// Get the received command
    pub fn get(&self) -> &Cmd {
        &self.inner
    }

    /// Respond to the receieved command
    pub fn respond(self, message: Msg) {
        let mut command = self;

        unsafe {
            // Set response message
            if let Some(ref mut data) = (*command.internal).data {
                data.message = ManuallyDrop::new(message);
            }

            // Release control to commander
            (*command.internal)
                .owner
                .0
                .store(COMMANDER, Ordering::Release);
            ManuallyDrop::take(&mut command.waker).wake();

            // Manual drop of inner
            let _ = ManuallyDrop::take(&mut command.inner);
        }

        // Forget self
        mem::forget(command);
    }

    /// Respond by closing the channel.
    pub fn close(self, messenger: Messenger<Cmd, Msg>) {
        let mut command = self;

        unsafe {
            // Free messenger
            let _ = messenger;
            // Manual drop of inner
            let _ = ManuallyDrop::take(&mut command.inner);
            // Release control to commander
            let waker = ManuallyDrop::take(&mut command.waker);
            // Notify commander
            (*command.internal)
                .owner
                .0
                .store(COMMANDER, Ordering::Release);
            waker.wake();
        }

        // Forget self
        mem::forget(command);
    }
}

/// Communication from the [`Messenger`].
#[must_use]
#[derive(Debug)]
pub struct Message<Cmd, Msg> {
    waker: ManuallyDrop<Waker>,
    inner: ManuallyDrop<Msg>,
    internal: *mut Internal<Cmd, Msg>,
}

impl<Cmd, Msg> Message<Cmd, Msg> {
    /// Get the received message
    pub fn get(&self) -> &Msg {
        &self.inner
    }

    /// Respond to the receieved message
    pub fn respond(self, command: Cmd) {
        let mut message = self;

        unsafe {
            // Set response command
            if let Some(ref mut data) = (*message.internal).data {
                data.command = ManuallyDrop::new(command);
            }

            // Release control to messenger
            (*message.internal)
                .owner
                .0
                .store(MESSENGER, Ordering::Release);
            ManuallyDrop::take(&mut message.waker).wake();

            // Manual drop of inner
            let _ = ManuallyDrop::take(&mut message.inner);
        }

        // Forget self
        mem::forget(message);
    }

    /// Respond by closing the channel.
    pub fn close(self, commander: Commander<Cmd, Msg>) {
        let mut message = self;

        unsafe {
            // Free commander
            let _ = commander;
            // Manual drop of inner
            let _ = ManuallyDrop::take(&mut message.inner);
            // Release control to messenger
            let waker = ManuallyDrop::take(&mut message.waker);
            // Notify messenger
            (*message.internal)
                .owner
                .0
                .store(MESSENGER, Ordering::Release);
            waker.wake();
        }

        // Forget self
        mem::forget(message);
    }
}
