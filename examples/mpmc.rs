use whisk::{Channel, Stream};

fn main() {
    let executor = pasts::Executor::default();
    let channel = Stream::from(Channel::new());
    for _ in 0..24 {
        let channel = channel.clone();
        std::thread::spawn(|| {
            pasts::Executor::default().spawn(async move {
                println!("Sending...");
                channel.send(Some(1)).await;
                let count = Stream::strong_count(&channel);
                println!("Sent {count}");
                if count <= 2 {
                    channel.send(None).await;
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
