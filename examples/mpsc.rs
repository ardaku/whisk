use std::sync::Arc;

use whisk::{Channel, Queue};

fn main() {
    let executor = pasts::Executor::default();
    let channel = Channel::new();
    for _ in 0..24 {
        let channel = channel.clone();
        std::thread::spawn(|| {
            pasts::Executor::default().spawn(async move {
                println!("Sending...");
                channel.send(Some(1)).await;
            })
        });
    }
    let queue: Arc<Queue<_>> = channel.into();
    executor.spawn(async move {
        let mut c = 0;
        while let Some(v) = queue.recv().await {
            println!("Received one.");
            c += v;
            if Arc::strong_count(&queue) <= 1 {
                break;
            }
        }
        println!("Received all.");
        assert_eq!(c, 24);
    });
}
