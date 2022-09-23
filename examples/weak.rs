use whisk::{Chan, Channel, WeakChan};
use std::sync::Arc;

// Call into executor of your choice
fn main() {
    pasts::Executor::default().spawn(async {
        let chan: Chan = Channel::new();
        let _weak_chan: WeakChan = Arc::downgrade(&chan);
    })
}
