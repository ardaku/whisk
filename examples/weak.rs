use std::sync::Arc;

use whisk::{Chan, Channel, WeakChan};

// Call into executor of your choice
fn main() {
    pasts::Executor::default().spawn(async {
        let chan: Chan = Chan::from(Channel::new());
        let _weak_chan: WeakChan = Arc::downgrade(&chan);
    })
}
