//! SPSC implementation

use alloc::boxed::Box;
use core::{
    mem,
    pin::Pin,
    task::{Context, Poll},
};

use crate::chan::{Receiving, Sender, Sending};

/// Handle to a worker, sends tasks
#[derive(Debug)]
#[repr(transparent)]
pub struct Worker<T: Send>(pub(crate) Sender<Option<T>>);

impl<T: Send> Worker<T> {
    /// Send next task to worker
    pub fn send(self, message: T) -> Message<T> {
        let sender: Sender<Option<T>> = unsafe { mem::transmute(self) };
        Message(sender.send(Some(message)))
    }

    /// Send message to stop the worker
    pub fn stop(self) -> Message<T> {
        let sender: Sender<Option<T>> = unsafe { mem::transmute(self) };
        Message(sender.send(None))
    }
}

impl<T: Send> Drop for Worker<T> {
    fn drop(&mut self) {
        panic!("Worker dropped without sending");
    }
}

/// Message sending future / notifier
#[derive(Debug)]
pub struct Message<T: Send>(Sending<Option<T>>);

impl<T: Send> Message<T> {
    /// Run an implementation of poll/poll_next for a fused future/notifier.
    pub fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Worker<T>> {
        let sending =
            unsafe { Pin::new_unchecked(&mut self.get_unchecked_mut().0) };

        sending.poll_next(cx).map(|channel| {
            let channel = Box::leak(channel);
            Worker(Sender(channel))
        })
    }
}

/// Handle to a tasker, receives tasks
#[derive(Debug)]
pub struct Tasker<T: Send>(pub(crate) Receiving<Option<T>>);

impl<T: Send> Tasker<T> {
    /// Run an implementation of poll/poll_next for a fused future/notifier.
    pub fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<T>> {
        let recving = unsafe { Pin::new_unchecked(&mut self.0) };

        recving.poll_next(cx)
    }
}
