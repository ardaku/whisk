use alloc::boxed::Box;
use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    ptr,
    sync::atomic::{
        AtomicPtr, AtomicUsize,
        Ordering::{Relaxed, SeqCst},
    },
    task::Waker,
};

/// Status of wake node
#[repr(usize)]
enum WakeState {
    /// Waiting to be re-allocated
    Garbage = 0,
    /// Waiting for registration
    Empty = 1,
    /// Ready to be awoken (contains waker)
    Ready = 2,
    /// 1 task can register at a time (in the process of getting a waker)
    Registering = 3,
    /// Multiple tasks could be waking (in the process of waking, losing waker)
    Waking = 4,
    /// Registration is being canceled by waking
    Canceling = 5,
    /// Waiting to become garbage
    Freeing = 6,
}

struct WakeNode {
    /// Atomic `WakeState` for waker
    state: AtomicUsize,
    /// Waker and a fallback waker
    waker: UnsafeCell<MaybeUninit<Waker>>,
}

impl WakeNode {
    /// Try to allocate wake node
    fn allocate(&self) -> Result<*const WakeNode, ()> {
        self.state
            .compare_exchange(
                WakeState::Garbage as usize,
                WakeState::Empty as usize,
                SeqCst,
                SeqCst,
            )
            .map_err(|_| ())
            .map(|_| -> *const WakeNode { self })
    }

    /// Register a new waker
    ///
    /// Slots can be Empty, Ready or Waking (If Waking, wakes immediately)
    fn register(&self, waker: Waker) {
        // Attempt to clear first slot and begin registering
        let r = self
            .state
            .fetch_update(SeqCst, SeqCst, |state| match state {
                // Switch to registering state
                x if x == WakeState::Empty as usize => {
                    Some(WakeState::Registering as usize)
                }
                // Switch to registering state, dropping previous waker
                x if x == WakeState::Ready as usize => {
                    Some(WakeState::Registering as usize)
                }
                // Contention with waking, re-wake immediately
                x if x == WakeState::Waking as usize => None,
                _ => unreachable!(),
            });

        // Set waker and mark ready
        match r {
            Ok(prev) => {
                // Drop before overwriting
                if prev == WakeState::Ready as usize {
                    unsafe { (*self.waker.get()).assume_init_drop() }
                }

                // Use first waker slot and set to ready for waking
                unsafe { *self.waker.get() = MaybeUninit::new(waker) };

                // Finish, checking if canceled
                let r =
                    self.state.fetch_update(
                        SeqCst,
                        SeqCst,
                        |state| match state {
                            // Switch to registering state
                            x if x == WakeState::Registering as usize => {
                                Some(WakeState::Ready as usize)
                            }
                            // Switch to registering state
                            x if x == WakeState::Canceling as usize => None,
                            _ => unreachable!(),
                        },
                    );
                if r.is_err() {
                    unsafe { (*self.waker.get()).assume_init_read().wake() }
                    self.state.store(WakeState::Empty as usize, SeqCst);
                }
            }
            Err(_) => waker.wake(),
        }
    }

    /// Try to wake this node
    ///
    /// If already waking, won't wake again
    fn wake(&self) -> Result<(), ()> {
        let r = self
            .state
            .fetch_update(SeqCst, SeqCst, |state| match state {
                // Ready to be awoken
                x if x == WakeState::Ready as usize => {
                    Some(WakeState::Waking as usize)
                }
                // Currently registering, wake now
                x if x == WakeState::Registering as usize => {
                    Some(WakeState::Canceling as usize)
                }
                // Not wakeable
                _ => None,
            })
            .map_err(|_| ())?;
        if r == WakeState::Registering as usize {
            return Ok(());
        }

        // Take and wake the waker
        unsafe { (*self.waker.get()).assume_init_read().wake() };

        // Update state
        self.state
            .fetch_update(SeqCst, SeqCst, |state| match state {
                x if x == WakeState::Waking as usize => {
                    Some(WakeState::Empty as usize)
                }
                x if x == WakeState::Freeing as usize => {
                    Some(WakeState::Garbage as usize)
                }
                _ => unreachable!(),
            })
            .map(|_| ())
            .map_err(|_| ())
    }

    /// Free this wake node (must be done on registration task)
    fn free(&self) {
        match self.state.swap(WakeState::Freeing as usize, SeqCst) {
            x if x == WakeState::Empty as usize => {}
            x if x == WakeState::Ready as usize => {
                unsafe { (*self.waker.get()).assume_init_drop() };
            }
            x if x == WakeState::Waking as usize => return,
            _ => unreachable!(),
        }

        self.state.store(WakeState::Garbage as usize, SeqCst);
    }
}

/// Handle to an optional waker node
pub(crate) struct WakeHandle(*const WakeNode);

unsafe impl Send for WakeHandle {}
unsafe impl Sync for WakeHandle {}

impl WakeHandle {
    /// Create a new mock handle
    pub(crate) fn new() -> Self {
        Self(ptr::null())
    }

    /// Register a waker
    pub(crate) fn register(&mut self, wl: &WakeList, waker: Waker) {
        // Allocate a waker if needed
        if self.0.is_null() {
            self.0 = wl.allocate();
        }

        // Register the waker
        unsafe { (*self.0).register(waker) }
    }
}

impl Drop for WakeHandle {
    fn drop(&mut self) {
        // Unregister the waker if set
        if !self.0.is_null() {
            unsafe { (*self.0).free() }
        }
    }
}

struct Node<T> {
    next: AtomicPtr<Node<T>>,
    data: T,
}

/// A `WakeList` stores an append-only atomic linked list of wakers
pub(crate) struct WakeList {
    // Root node of list of wakers
    root: AtomicPtr<Node<WakeNode>>,
    // Next one to try waking ("fairness" mechanism)
    next: AtomicPtr<Node<WakeNode>>,
}

impl Drop for WakeList {
    fn drop(&mut self) {
        let mut tmp = self.root.load(Relaxed);

        while !tmp.is_null() {
            let node = unsafe { Box::from_raw(tmp) };
            tmp = node.next.load(Relaxed);
        }
    }
}

impl WakeList {
    /// Create a new empty wake list
    pub(crate) const fn new() -> Self {
        Self {
            root: AtomicPtr::new(ptr::null_mut()),
            next: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// Attempt to wake one waker.
    ///
    /// If no wakers are registered, doesn't do anything.
    pub(crate) fn wake_one(&self) {
        // Start from next pointer into list
        let next = self.next.load(SeqCst);
        let mut tmp = next;
        while !tmp.is_null() {
            let next = unsafe { (*tmp).next.load(Relaxed) };
            if unsafe { (*tmp).data.wake().is_ok() } {
                self.next.store(next, SeqCst);
                return;
            }
            tmp = next;
        }

        // Start back at beginning of list
        tmp = self.root.load(SeqCst);
        while tmp != next {
            let next = unsafe { (*tmp).next.load(Relaxed) };
            if unsafe { (*tmp).data.wake().is_ok() } {
                self.next.store(next, SeqCst);
                return;
            }
            tmp = next;
        }

        ::std::println!("skiosp");
    }

    /// Allocate a new `WakeNode`
    fn allocate(&self) -> *const WakeNode {
        // Go through list to see if unused existing allocation to use
        let mut tmp = self.root.load(SeqCst);
        while !tmp.is_null() {
            if let Ok(wn) = unsafe { (*tmp).data.allocate() } {
                return wn;
            }
            tmp = unsafe { (*tmp).next.load(Relaxed) };
        }

        // Push to front
        let data = WakeNode {
            state: AtomicUsize::new(WakeState::Empty as usize),
            waker: UnsafeCell::new(MaybeUninit::uninit()),
        };
        let mut root = self.root.load(SeqCst);
        let next = AtomicPtr::new(self.root.load(SeqCst));
        let node = Box::into_raw(Box::new(Node { next, data }));
        while let Err(r) =
            self.root.compare_exchange(root, node, SeqCst, Relaxed)
        {
            root = r;
            unsafe { (*node).next = AtomicPtr::new(root) };
        }
        unsafe { &(*node).data }
    }
}
