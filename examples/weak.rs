use std::sync::Arc;

use whisk::Channel;

// Call into executor of your choice
fn main() {
    pasts::Executor::default().spawn(async {
        let chan: Channel = Channel::new();
        let _weak_chan = Arc::downgrade(&chan.into_inner());
    })
}
