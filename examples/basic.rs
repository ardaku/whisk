use whisk::{Channel, Sender, Tasker};

enum Cmd {
    /// Tell messenger to add
    Add(u32, u32, Sender<u32>),
}

async fn worker_main(mut tasker: Tasker<Cmd>) {
    while let Some(command) = (&mut tasker).await {
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
    let (worker, tasker) = Channel::new().spsc();
    let worker_thread = std::thread::spawn(move || {
        pasts::Executor::default()
            .spawn(Box::pin(async move { worker_main(tasker).await }))
    });

    // Do an addition
    println!("Sending command…");
    let (send, recv) = Channel::new().oneshot();
    let worker = worker.send(Cmd::Add(43, 400, send)).await;
    println!("Receiving response…");
    let response = recv.recv().await;
    assert_eq!(response, 443);

    // Tell worker to stop
    println!("Stopping worker…");
    worker.stop().await;
    println!("Waiting for worker to stop…");

    worker_thread.join().unwrap();
    println!("Worker thread joined");
}

// Call into executor of your choice
fn main() {
    pasts::Executor::default().spawn(Box::pin(tasker_main()))
}
