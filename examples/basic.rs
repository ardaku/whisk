use whisk::{Chan, Channel, Stream};

enum Cmd {
    /// Tell messenger to add
    Add(u32, u32, Chan<u32>),
}

async fn worker_main(commands: Stream<Cmd>) {
    while let Some(command) = commands.recv().await {
        println!("Worker receiving command");
        match command {
            Cmd::Add(a, b, s) => s.send(a + b).await,
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
            let future = async move { worker_main(channel).await };
            pasts::Executor::default().spawn(future)
        })
    };

    // Do an addition
    println!("Sending command…");
    let oneshot = Channel::new();
    channel.send(Some(Cmd::Add(43, 400, oneshot.clone()))).await;
    println!("Receiving response…");
    let response = oneshot.recv().await;
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
    pasts::Executor::default().spawn(tasker_main())
}
