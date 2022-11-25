use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};

use whisk::{Channel, Queue};

static ONCE: AtomicBool = AtomicBool::new(true);

fn main() {
    let executor = pasts::Executor::default();
    let channel = Channel::new();
    for _ in 0..24 {
        let channel = channel.clone();
        std::thread::spawn(|| {
            pasts::Executor::default().spawn(async move {
                println!("Sending...");
                channel.send(Some(1)).await;
                let weak: Weak<Queue<_>> = Arc::downgrade(&channel.into());
                if Weak::strong_count(&weak) == 1 {
                    if ONCE.fetch_and(false, Ordering::Relaxed) {
                        weak.upgrade().unwrap().send(None).await;
                    }
                }
            })
        });
    }
    executor.spawn(async move {
        let mut c = 0;
        while let Some(v) = channel.recv().await {
            println!("Received one.");
            c += v;
        }
        println!("Received all.");
        assert_eq!(c, 24);
    });
}
