//! A simple and fast two-way sync/async channel.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Waker},
    future::Future,
    mem::ManuallyDrop,
};

struct Owner(AtomicBool);

const COMMANDER: Owner = Owner(AtomicBool::new(false));
const MESSENGER: Owner = Owner(AtomicBool::new(true));

union Data<Command, Message> {
    /// Command for child
    command: ManuallyDrop<Command>,
    /// Message for parent
    message: ManuallyDrop<Message>,
}

struct Internal<Command, Message> {
    /// Who owns the data
    owner: Owner,
    /// Waker of other task
    waker: Option<Waker>,
    /// Union, because discriminant is `owner` field.
    data: Option<Data<Command, Message>>,
}

/// A commander tells the messenger what to do.
pub struct Commander<Command, Message>(*mut Internal<Command, Message>);

unsafe impl<Command: Send, Message: Send> Send for Commander<Command, Message> {}

impl<Command, Message> Drop for Commander<Command, Message> {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            // (*self.0).commander_send(None);
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
            // (*self.0).messenger_send(None);
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
        owner: MESSENGER,
        waker: None,
        data: Some(Data { message: ManuallyDrop::new(ready) }),
    };
    let internal = Box::leak(Box::new(internal));

    (Commander(internal), Messenger(internal))
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
