//! A simple and fast two-way sync/async channel.
//!
//! The idea is based on a concept of calling a function on a different task.
//!
//! # Getting Started
//! 

use std::{
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Waker},
    future::Future,
    mem::ManuallyDrop,
};

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
pub struct Commander<Command, Message>(*mut Internal<Command, Message>);

unsafe impl<Command: Send, Message: Send> Send for Commander<Command, Message> {}

impl<Command, Message> Drop for Commander<Command, Message> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            // Wait for message from messenger
            while (*self.0).owner.0.load(Ordering::Acquire) != COMMANDER {}

            if (*self.0).leased {
                // Panic so that `Command` can't use-after-free.
                //
                // Reaching this should always be considered a bug in the
                // calling code
                panic!("Cannot drop `Commander` before `Command`");
            } else if (*self.0).data.is_some() {
                // Let other messenger know commander is gone
                (*self.0).data = None;
            } else {
                // Messenger is gone, so the commander has to clean up
                let _ = Box::from_raw(self.0);
            }
        }
    }
}

/// A messenger reports the results of tasks the commander assigned.
pub struct Messenger<Command, Message>(*mut Internal<Command, Message>);

unsafe impl<Command: Send, Message: Send> Send for Messenger<Command, Message> {}

impl<Command, Message> Drop for Messenger<Command, Message> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            // Wait for command from commander
            while (*self.0).owner.0.load(Ordering::Acquire) != MESSENGER {}

            if (*self.0).leased {
                // Panic so that `Message` can't use-after-free.
                //
                // Reaching this should always be considered a bug in the
                // calling code
                panic!("Cannot drop `Messenger` before `Message`");
            } else if (*self.0).data.is_some() {
                // Let other commander know messenger is gone
                (*self.0).data = None;
            } else {
                // Commander is gone, so the messenger has to clean up
                let _ = Box::from_raw(self.0);
            }
        }
    }
}

/// Create an asynchronous 2-way channel between two tasks.
///
/// Should be called on commander task.  Usually, the first message from the
/// messenger task will be Ready for commands.
#[inline(always)]
pub fn channel<Command, Message>(ready: Message) -> (Commander<Command, Message>, Messenger<Command, Message>) {
    let internal = Internal {
        owner: Owner(AtomicBool::new(MESSENGER)),
        leased: false,
        waker: None,
        data: Some(Data { message: ManuallyDrop::new(ready) }),
    };
    let internal = Box::leak(Box::new(internal));

    (Commander(internal), Messenger(internal))
}

/// Communication from the [`Commander`].
#[must_use]
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
            (*command.internal).owner.0.store(COMMANDER, Ordering::Release);
            ManuallyDrop::take(&mut command.waker).wake();

            // Manual drop of inner
            let _ = ManuallyDrop::take(&mut command.inner);
        }

        // Forget self
        std::mem::forget(command);
    }
}

/// Communication from the [`Messenger`].
#[must_use]
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
            (*message.internal).owner.0.store(MESSENGER, Ordering::Release);
            ManuallyDrop::take(&mut message.waker).wake();

            // Manual drop of inner
            let _ = ManuallyDrop::take(&mut message.inner);
        }

        // Forget self
        std::mem::forget(message);
    }
}

/*
impl<Message> Future for Channel<Message, false> {
    type Output = Message;

    fn poll(self: Pin<&mut self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Child -> Parent
        if Owner(self.0.owner.0.load(Ordering::Acquire)) == MESSENGER {
            let waker = self.0.waker.take();
            self.0.waker = Some(cx.waker().clone());
            self.0.ready.store(COMMANDER.0, Ordering::Release);
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}*/

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
