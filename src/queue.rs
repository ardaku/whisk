use core::{
    cell::Cell,
    future::{self, Future},
    pin::Pin,
    task::{Context, Poll},
};

use crate::{mutex::Mutex, wake_list::WakeHandle};

/// A `Queue` can send messages to itself, and can be shared between threads
/// and tasks.
///
/// Implemented as a multi-producer/multi-consumer queue of size 1.
pub struct Queue<T = (), U: ?Sized = ()> {
    /// Data in transit
    pub(crate) data: Mutex<T>,
    /// User data
    pub(crate) user: U,
}

impl<T, U: ?Sized> core::fmt::Debug for Queue<T, U> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Queue").finish_non_exhaustive()
    }
}

impl<T, U: ?Sized> core::ops::Deref for Queue<T, U> {
    type Target = U;

    fn deref(&self) -> &Self::Target {
        &self.user
    }
}

impl<T, U: ?Sized + Default> Default for Queue<T, U> {
    fn default() -> Self {
        Self::with(U::default())
    }
}

impl<T> Queue<T> {
    /// Create a new queue.
    #[inline]
    pub const fn new() -> Self {
        Self::with(())
    }
}

impl<T, U> Queue<T, U> {
    /// Create a new queue with associated data.
    #[inline]
    pub const fn with(user_data: U) -> Self {
        Self {
            data: Mutex::new(),
            user: user_data,
        }
    }
}

impl<T, U: ?Sized> Queue<T, U> {
    /// Send a message on this queue.
    #[inline(always)]
    pub async fn send(&self, message: T) {
        Message(self, Cell::new(Some(message)), WakeHandle::new()).await
    }

    /// Receive a message from this queue.
    #[inline(always)]
    pub async fn recv(&self) -> T {
        let mut wh = WakeHandle::new();
        future::poll_fn(|cx| self.data.take(cx, &mut wh)).await
    }
}

/// A message in the process of being sent over a [`Queue`].
struct Message<'a, T, U: ?Sized>(&'a Queue<T, U>, Cell<Option<T>>, WakeHandle);

#[allow(unsafe_code)]
impl<T, U: ?Sized> Message<'_, T, U> {
    #[inline(always)]
    fn pin_get_wh(self: Pin<&mut Self>) -> Pin<&mut WakeHandle> {
        // This is okay because `1` is pinned when `self` is.
        unsafe { self.map_unchecked_mut(|s| &mut s.2) }
    }
}

impl<T, U: ?Sized> Future for Message<'_, T, U> {
    type Output = ();

    #[inline]
    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let mut wh = WakeHandle::new();
        core::mem::swap(&mut wh, self.as_mut().pin_get_wh().get_mut());

        let ret = self.0.data.store(&self.1, cx, &mut wh);

        core::mem::swap(&mut wh, self.as_mut().pin_get_wh().get_mut());

        ret
    }
}
