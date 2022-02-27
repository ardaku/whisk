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
//! }
//!
//! enum Cmd {
//!     /// Tell messenger to quit
//!     Exit,
//! }
//!
//! async fn messenger_task(mut messenger: Messenger<Cmd, Msg>) {
//!     // Some work
//!     println!("Doing initialization work....");
//!     // Receive command from commander
//!     while let Some(command) = (&mut messenger).await {
//!         match command.get() {
//!             Cmd::Exit => {
//!                 println!("Messenger received exit, shutting down....");
//!                 command.close(messenger);
//!                 return;
//!             }
//!         }
//!     }
//!     unreachable!()
//! }
//!
//! async fn commander_task() {
//!     let (mut commander, messenger) = whisk::channel(Msg::Ready).await;
//!     let messenger = messenger_task(messenger);
//!
//!     // Start task on another thread
//!     std::thread::spawn(|| pasts::block_on(messenger));
//!
//!     // wait for Ready message, and respond with Exit command
//!     println!("Waiting messages....");
//!     while let Some(message) = (&mut commander).await {
//!         match message.get() {
//!             Msg::Ready => {
//!                 println!("Received ready, telling messenger to exit....");
//!                 message.respond(Cmd::Exit)
//!             }
//!         }
//!     }
//!     println!("Messenger has exited, now too shall the commander");
//! }
//!
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

macro_rules! println {
    ($($arg:tt)*) => ({
        std::println!($($arg)*);
        std::thread::sleep(std::time::Duration::from_millis(200));
    })
}

use std::println;

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
    /// True if ready message has not been sent yet
    first: bool,
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

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        println!("Polling commander");
        unsafe {
            if (*self.0).owner.0.load(Ordering::Acquire) == COMMANDER {
                if (*self.0).leased {
                    std::process::exit(1); //panic!("Didn't respond before next .await");
                }

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
                    (*self.0).leased = true;

                    println!("Commander received a message!");
                    Poll::Ready(Some(message))
                } else {
                    println!("Commander cannot find messenger!");
                    Poll::Ready(None)
                }
            } else {
                println!("Commander still waiting....");
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
                std::process::exit(1);// panic!("Cannot drop `Commander` before `Command`");
            } else if (*self.0).data.is_some() {
                // Let other messenger know commander is gone
                (*self.0).data = None;
                println!("STORE: Freeing commander, messenger only now");
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

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        println!("Polling messenger");

        unsafe {
            if (*self.0).owner.0.load(Ordering::Acquire) == MESSENGER {
                if (*self.0).leased {
                    std::process::exit(1);// panic!("Didn't respond before next .await");
                }

                if (*self.0).first {
                    let waker = (*self.0).waker.take().unwrap();

                    (*self.0).first = false;
                    (*self.0).owner.0.store(COMMANDER, Ordering::Release);
                    (*self.0).waker = Some(cx.waker().clone());

                    waker.wake();

                    return Poll::Pending
                }

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
                    (*self.0).leased = true;

                    println!("Messenger received a command!");
                    Poll::Ready(Some(command))
                } else {
                    println!("Messenger can not find commander!");
                    Poll::Ready(None)
                }
            } else {
                println!("Messenger still waiting...");
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
                std::process::exit(1);//panic!("Cannot drop `Messenger` before `Message`");
            } else if (*self.0).data.is_some() {
                // Let other commander know messenger is gone
                (*self.0).data = None;
                println!("STORE: Freeing messenger, commander only now");
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

impl<Cmd: Unpin, Msg: Unpin> Future for Channel<Cmd, Msg> {
    type Output = (Commander<Cmd, Msg>, Messenger<Cmd, Msg>);

    #[inline(always)]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let waker = cx.waker().clone();
        let internal = Internal {
            owner: Owner(AtomicBool::new(MESSENGER)),
            leased: false,
            first: true,
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
pub fn channel<Cmd: Unpin, Msg: Unpin>(
    ready: Msg,
) -> impl Future<Output = (Commander<Cmd, Msg>, Messenger<Cmd, Msg>)>
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
    #[inline(always)]
    pub fn get(&self) -> &Cmd {
        &self.inner
    }

    /// Respond to the receieved command
    #[inline(always)]
    pub fn respond(self, message: Msg) {
        let mut command = self;

        unsafe {
            // Set response message
            println!("Mesenger responding");
            if let Some(ref mut data) = (*command.internal).data {
                data.message = ManuallyDrop::new(message);
            } else {
                println!("Mesenger failed to respond");
            }
            println!("Mesenger responded");

            // Release control to commander
            (*command.internal).leased = false;
            println!("STORE: Messenger responding to commander");
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
    #[inline(always)]
    pub fn close(self, messenger: Messenger<Cmd, Msg>) {
        let mut command = self;

        unsafe {
            // Release control to commander
            (*command.internal).leased = false;
            // Drop messenger, notify commander
            let _ = messenger;
            // Manual drop of inner
            let _ = ManuallyDrop::take(&mut command.inner);
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
    #[inline(always)]
    pub fn get(&self) -> &Msg {
        &self.inner
    }

    /// Respond to the receieved message
    #[inline(always)]
    pub fn respond(self, command: Cmd) {
        let mut message = self;

        unsafe {
            // Set response command
            println!("Commander sending response");
            if let Some(ref mut data) = (*message.internal).data {
                data.command = ManuallyDrop::new(command);
            } else {
                println!("Commander failed to send response");
            }
            println!("Commander sent response");

            // Release control to messenger
            (*message.internal).leased = false;
            println!("STORE: Commander responding to messenger");
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
    #[inline(always)]
    pub fn close(self, commander: Commander<Cmd, Msg>) {
        let mut message = self;

        unsafe {
            // Release control to messenger
            (*message.internal).leased = false;
            // Drop commander, notify messenger
            let _ = commander;
            // Manual drop of inner
            let _ = ManuallyDrop::take(&mut message.inner);
        }

        // Forget self
        mem::forget(message);
    }
}
