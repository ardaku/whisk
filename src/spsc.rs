//! SPSC implementation

use alloc::boxed::Box;
use core::{
    future::Future,
    mem,
    pin::Pin,
    task::{
        Context,
        Poll::{self, Ready},
    },
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
    pub async fn stop(self) {
        let sender: Sender<Option<T>> = unsafe { mem::transmute(self) };
        sender.send(None).await;
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

impl<T: Send> Future for Message<T> {
    type Output = Worker<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_next(cx)
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
        let mut recving = unsafe { Pin::new_unchecked(&mut self.0) };
        let message = recving.as_mut().poll_next_reuse(cx);

        if let Ready(None) = message {
            recving.poll_next_unuse();
        }

        message
    }
}

impl<T: Send> Future for Tasker<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_next(cx)
    }
}
