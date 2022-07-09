use whisk::{Channel, Sender, Tasker, Worker};

enum Cmd {
    /// Tell messenger to add
    Add(u32, u32, Sender<u32>),
    /// Tell messenger to quit
    Stop,
}

async fn worker(tasker: Tasker<Cmd>) {
    loop {
        println!("Worker receiving command");
        match tasker.recv_next().await {
            Cmd::Add(a, b, s) => s.send(a + b),
            Cmd::Stop => break,
        }
    }

    println!("Worker stopping…");
    drop(tasker);
    println!("Worker stopped…");
}

async fn tasker() {
    // Create worker on new thread
    println!("Spawning worker…");
    let mut worker_thread = None;
    let worker = Worker::new(|tasker| {
        worker_thread = Some(std::thread::spawn(move || {
            pasts::Executor::default()
                .spawn(Box::pin(async move { worker(tasker).await }))
        }));
    });

    // Do an addition
    println!("Sending command…");
    let (send, recv) = Channel::pair();
    worker.send(Cmd::Add(43, 400, send));
    println!("Receiving response…");
    let response = recv.recv().await;
    assert_eq!(response, 443);

    // Tell worker to stop
    println!("Stopping worker…");
    worker.send(Cmd::Stop);

    // Close channel
    println!("Closing channel…");
    drop(worker);
    println!("Closed channel…");

    worker_thread.unwrap().join().unwrap();
    println!("Worker thread joined");
}

// Call into executor of your choice
fn main() {
    pasts::Executor::default().spawn(Box::pin(tasker()))
}
