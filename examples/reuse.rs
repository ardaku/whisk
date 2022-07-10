use whisk::{Channel, Sender, Tasker, Worker};

enum Cmd {
    /// Tell messenger to add
    Add(u32, u32, Sender<u32>),
}

async fn worker(tasker: Tasker<Cmd>) {
    while let Some(command) = tasker.recv_next().await {
        match command {
            Cmd::Add(a, b, s) => s.send(a + b).await,
        }
    }
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
    let (send, recv) = Channel::pair();
    worker.send(Cmd::Add(43, 400, send)).await;
    let (mut _resp, mut chan) = recv.recv_chan().await;
    for _ in 1..256 {
        let (send, recv) = chan.to_pair();
        worker.send(Cmd::Add(43, 400, send)).await;
        (_resp, chan) = recv.recv_chan().await;
    }

    // Tell worker to stop
    println!("Dropping worker…");
    worker.stop().await;
    println!("Waiting for worker to stop…");

    worker_thread.unwrap().join().unwrap();
    println!("Worker thread joined");
}

// Call into executor of your choice
fn main() {
    pasts::Executor::default().spawn(Box::pin(tasker()))
}
