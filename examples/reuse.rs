use whisk::Channel;

enum Cmd {
    /// Tell messenger to add
    Add(u32, u32, Channel<u32>),
}

async fn worker(channel: Channel<Option<Cmd>>) {
    while let Some(command) = channel.recv().await {
        println!("Worker receiving command");
        match command {
            Cmd::Add(a, b, s) => s.send(a + b).await,
        }
    }

    println!("Worker stopping…");
}

// Call into executor of your choice
#[async_main::async_main]
async fn main(_spawner: impl async_main::Spawn) {
    // Create worker on new thread
    println!("Spawning worker…");
    let channel = Channel::new();
    let worker_task = worker(channel.clone());
    let worker_thread =
        std::thread::spawn(|| pasts::Executor::default().block_on(worker_task));

    // Do an addition
    let oneshot = Channel::new();
    for _ in 0..32 {
        println!("Sending command…");
        channel.send(Some(Cmd::Add(43, 400, oneshot.clone()))).await;
        println!("Receiving response…");
        let response = oneshot.recv().await;
        assert_eq!(response, 443);
    }

    // Tell worker to stop
    println!("Stopping worker…");
    channel.send(None).await;
    println!("Waiting for worker to stop…");

    worker_thread.join().unwrap();
    println!("Worker thread joined");
}
