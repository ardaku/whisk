use whisk::Channel;

enum Cmd {
    /// Tell messenger to add
    Add(u32, u32, Channel<u32>),
}

async fn worker_main(mut channel: Channel<Option<Cmd>>) {
    while let Some(command) = channel.recv().await {
        println!("Worker receiving command");
        match command {
            Cmd::Add(a, b, s) => {
                s.send(a + b).await;
            }
        }
    }

    println!("Worker stopping…");
}

async fn tasker_main() {
    // Create worker on new thread
    println!("Spawning worker…");
    let channel = Channel::new();
    let worker_thread = {
        let channel = channel.clone();
        std::thread::spawn(move || {
            pasts::Executor::default()
                .spawn(Box::pin(async move { worker_main(channel).await }))
        })
    };

    // Do an addition
    println!("Sending command…");
    let oneshot = Channel::new();
    channel.send(Some(Cmd::Add(43, 400, oneshot.clone()))).await;
    println!("Receiving response…");
    let response = oneshot.await;
    assert_eq!(response, 443);

    // Tell worker to stop
    println!("Stopping worker…");
    channel.send(None).await;
    println!("Waiting for worker to stop…");

    worker_thread.join().unwrap();
    println!("Worker thread joined");
}

// Call into executor of your choice
fn main() {
    pasts::Executor::default().spawn(Box::pin(tasker_main()))
}
