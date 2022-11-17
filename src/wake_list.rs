use alloc::boxed::Box;
use core::{
    cell::UnsafeCell,
    num::NonZeroUsize,
    ptr,
    sync::atomic::{
        AtomicBool, AtomicPtr, AtomicUsize,
        Ordering::{Acquire, Relaxed, Release, SeqCst},
    },
    task::Waker,
};

pub(crate) struct WakeHandle(usize);

/// A `WakeList` stores append-only atomic linked lists of wakers and garbage
pub(crate) struct WakeList {
    // List of wakers (eventually will be able to use `AtomicMaybeWaker`)
    wakers: AtomicLinkedList<(AtomicBool, UnsafeCell<Option<Waker>>)>,
    // List of garbage
    garbage: AtomicLinkedList<AtomicUsize>,
    // Next handle ID / length of wakers list
    size: AtomicUsize,
    // Next one to wake
    next: AtomicUsize,
}

impl WakeList {
    /// Create a new wake list
    pub(crate) const fn new() -> Self {
        let wakers = AtomicLinkedList::new();
        let garbage = AtomicLinkedList::new();
        let size = AtomicUsize::new(0);
        let next = AtomicUsize::new(0);

        Self {
            wakers,
            garbage,
            size,
            next,
        }
    }

    /// Register a waker
    pub(crate) fn register(
        &self,
        waker: impl Into<Option<Waker>>,
    ) -> WakeHandle {
        let waker = waker.into();

        // Check garbage for reuse
        let garbage = 'garbage: {
            for garbage in self.garbage.iter() {
                if let Some(wh) =
                    NonZeroUsize::new(garbage.fetch_and(0, Relaxed))
                {
                    let index = usize::from(wh) - 1;
                    let wakey = self.wakers.iter().nth(index).unwrap();
                    // Only if non-contended (AtomicBool could be true)
                    if !wakey.0.load(Acquire) {
                        break 'garbage Some((wakey, index));
                    }
                }
            }
            None
        };

        // Add waker to the list and get its index as a handle
        let handle = if let Some((wakey, index)) = garbage {
            // Replace existing
            unsafe { *wakey.1.get() = waker };
            index
        } else {
            // If no garbage exists, push new pair
            self.wakers
                .push((AtomicBool::new(false), UnsafeCell::new(waker)));
            self.size.fetch_add(1, Relaxed)
        };

        WakeHandle(handle)
    }

    /// Re-register a `WakeHandle` with a new `Waker`.
    pub(crate) fn reregister(&self, handle: &mut WakeHandle, waker: Waker) {
        let wakey = self.wakers.iter().nth(handle.0).unwrap();

        if !wakey.0.fetch_or(true, Acquire) {
            // Non-contended, update
            unsafe { *wakey.1.get() = Some(waker) };
            wakey.0.fetch_and(false, Release);
        } else {
            // Contended, unregister and register again
            self.unregister(handle);
            *handle = self.register(waker);
        }
    }

    /// Clean up wake handle
    pub(crate) fn unregister(&self, handle: &mut WakeHandle) {
        // Add to garbage
        let handle = handle.0 + 1;

        // Go through existing slots, looking for an empty one
        for garbage in self.garbage.iter() {
            if garbage
                .compare_exchange(0, handle, Relaxed, Relaxed)
                .is_ok()
            {
                // Added to garbage successfully
                return;
            }
        }

        // Garbage is out of space, allocate new node
        self.garbage.push(AtomicUsize::new(handle));
    }

    /// Attempt to wake one task
    pub(crate) fn wake_one(&self) {
        // Starting index, goes until end then wraps around
        let which = self.next.load(Relaxed);
        let len = self.size.load(Relaxed).max(1);

        'untilone: {
            for wakey in self
                .wakers
                .iter()
                .skip(which)
                .chain(self.wakers.iter().take(which))
            {
                if !wakey.0.fetch_or(true, Acquire) {
                    if let Some(waker) = unsafe { (*wakey.1.get()).take() } {
                        waker.wake();
                        wakey.0.fetch_and(false, Release);
                        break 'untilone;
                    }
                    wakey.0.fetch_and(false, Release);
                }
            }
        }

        let _ = self
            .next
            .fetch_update(Relaxed, Relaxed, |old| Some((old + 1) % len));
    }
}

struct Node<T> {
    next: AtomicPtr<Node<T>>,
    data: T,
}

struct AtomicLinkedList<T> {
    root: AtomicPtr<Node<T>>,
}

impl<T> Drop for AtomicLinkedList<T> {
    fn drop(&mut self) {
        let mut tmp = self.root.load(Relaxed);

        loop {
            if tmp.is_null() {
                return;
            }
            let node = unsafe { Box::from_raw(tmp) };
            tmp = node.next.load(Relaxed);
        }
    }
}

impl<T> AtomicLinkedList<T> {
    const fn new() -> Self {
        let root = AtomicPtr::new(ptr::null_mut());

        Self { root }
    }

    fn push(&self, data: T) -> usize {
        let nul = ptr::null_mut();
        let next = AtomicPtr::new(nul);
        let node = Box::into_raw(Box::new(Node { next, data }));
        let mut count = 0;
        let mut ptr = self.root.load(Relaxed);

        // If empty, try to write at root node
        if ptr.is_null() {
            if let Err(p) =
                self.root.compare_exchange(nul, node, SeqCst, Relaxed)
            {
                // Handle contention, push next slot
                ptr = p;
                count += 1;
                while let Err(p) = unsafe {
                    (*ptr).next.compare_exchange(nul, node, SeqCst, Relaxed)
                } {
                    // Handle contention, push next slot
                    ptr = p;
                    count += 1;
                }
            }
            return count;
        }

        // Go to end of list (functionally unnecessary, but faster in theory)
        loop {
            let tmp = unsafe { (*ptr).next.load(Relaxed) };
            if tmp.is_null() {
                break;
            }
            ptr = tmp;
            count += 1;
        }

        // Push at end of list
        while let Err(p) =
            unsafe { (*ptr).next.compare_exchange(nul, node, SeqCst, Relaxed) }
        {
            // Handle contention, push next slot
            ptr = p;
            count += 1;
        }
        count
    }

    fn iter(&self) -> AtomicLinkedListIter<'_, T> {
        AtomicLinkedListIter {
            _all: self,
            next: self.root.load(Relaxed),
        }
    }
}

struct AtomicLinkedListIter<'a, T> {
    _all: &'a AtomicLinkedList<T>,
    next: *mut Node<T>,
}

impl<'a, T> Iterator for AtomicLinkedListIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        if self.next.is_null() {
            return None;
        }

        let ret: *const T = unsafe { &(*self.next).data };

        // Advance
        self.next = unsafe { (*self.next).next.load(Relaxed) };

        Some(unsafe { &*ret })
    }
}
