//! WakeList implementation

use alloc::boxed::Box;
use core::{
    sync::atomic::{AtomicUsize, Ordering},
    task::Waker,
};

use crate::{
    list::{DynList, List},
    tcms::Tcms,
};

pub(crate) struct RecvHandle(usize);

pub(crate) struct SendHandle(usize);

/// Lockless MPMC multi-waker
pub(crate) struct WakeList {
    /// List of wakers, with up to 2 on the stack before falling back to heap
    send: Tcms<DynList<(usize, Option<Waker>), 2>>,
    /// List of wakers, with up to 2 on the stack before falling back to heap
    recv: Tcms<DynList<(usize, Option<Waker>), 2>>,
    /// List of garbage, with up to 2 on the stack before falling back to heap
    garbage: Tcms<DynList<usize, 2>>,
    /// Next slot
    slot: AtomicUsize,
}

impl WakeList {
    /// Create a new lockless multi-waker
    pub(crate) fn new() -> Self {
        let send = Tcms::new();
        let recv = Tcms::new();
        let garbage = Tcms::new();
        let slot = AtomicUsize::new(0);

        Self {
            send,
            recv,
            garbage,
            slot,
        }
    }

    /// Set waker
    fn when_recv(&self, waker: Waker) -> RecvHandle {
        let id = self
            .garbage
            .with(|g| g.pop(), merge)
            .unwrap_or_else(|| self.slot.fetch_add(1, Ordering::Relaxed));
        self.recv.with(|list| list.push((id, Some(waker))), merge);
        RecvHandle(id)
    }

    /// Set waker
    fn when_send(&self, waker: Waker) -> SendHandle {
        let id = self
            .garbage
            .with(|g| g.pop(), merge)
            .unwrap_or_else(|| self.slot.fetch_add(1, Ordering::Relaxed));
        self.send.with(|list| list.push((id, Some(waker))), merge);
        SendHandle(id)
    }

    /// Free a send handle to be reused
    fn begin_free_send(&self, _handle: SendHandle) {
        todo!()
    }

    /// Free a recv handle to be reused
    fn begin_free_recv(&self, _handle: RecvHandle) {
        todo!()
    }

    /// Wake one waker
    fn begin_wake_one_send(&self) {
        todo!()
    }

    /// Wake all wakers
    fn begin_wake_all_send(&self) {
        todo!()
    }

    /// Wake one waker
    fn begin_wake_one_recv(&self) {
        todo!()
    }

    /// Wake all wakers
    fn begin_wake_all_recv(&self) {
        todo!()
    }
}

fn merge<T>(orig: &mut Box<DynList<T, 2>>, other: Box<DynList<T, 2>>) {
    orig.merge(other)
}
