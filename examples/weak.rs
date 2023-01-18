use std::sync::{Arc, Weak};

use whisk::{Channel, Queue};

// Call into executor of your choice
fn main() {
    pasts::Executor::default().block_on(async {
        let chan: Channel = Channel::new();
        let _weak_chan: Weak<Queue> = Arc::downgrade(&Arc::from(chan));
    })
}
