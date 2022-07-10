use whisk::{Channel, Sender, Tasker, Worker};
use std::time::Instant;
use pasts::prelude::*;
use dl_api::manual::DlApi;
use std::ffi::CStr;

enum Cmd {
    /// Tell messenger to get cosine
    Cos(f32, Sender<f32>),
}

async fn worker(tasker: Tasker<Cmd>) {
    while let Some(command) = tasker.recv_next().await {
        match command {
            Cmd::Cos(a, s) => s.send(libm::cosf(a)).await,
        }
    }
}

async fn worker_flume(tasker: flume::Receiver<Cmd>) {
    while let Ok(command) = tasker.recv_async().await {
        match command {
            Cmd::Cos(a, s) => s.send(libm::cosf(a)).await,
        }
    }
}

async fn tasker_multi() {
    // Create worker on new thread
    let mut worker_thread = None;
    let worker = Worker::new(|tasker| {
        worker_thread = Some(std::thread::spawn(move || {
            pasts::Executor::default()
                .spawn(Box::pin(async move { worker(tasker).await }))
        }));
    });

    let (send, recv) = Channel::pair();
    worker.send(Cmd::Cos(750.0, send)).await;
    let (mut _resp, mut chan) = recv.recv_chan().await;
    for _ in 1..=1024 {
        let (send, recv) = chan.to_pair();
        worker.send(Cmd::Cos(750.0, send)).await;
        (_resp, chan) = recv.recv_chan().await;
    }
    let now = Instant::now();
    for _ in 1..=1024*256 {
        let (send, recv) = chan.to_pair();
        worker.send(Cmd::Cos(750.0, send)).await;
        (_resp, chan) = recv.recv_chan().await;
    }
    let elapsed = now.elapsed() / (1024*256);
    println!("Whisk (2-thread): {:?}", elapsed);

    // Tell worker to stop
    worker.stop().await;
    worker_thread.unwrap().join().unwrap();
}

async fn tasker_single(executor: &Executor) {
    // Create worker on new thread
    let (task, join) = Channel::pair();
    let worker = Worker::new(|tasker| {
        executor.spawn(Box::pin(async move {
            worker(tasker).await;
            task.send(()).await;
        }))
    });

    let (send, recv) = Channel::pair();
    worker.send(Cmd::Cos(750.0, send)).await;
    let (mut _resp, mut chan) = recv.recv_chan().await;
    for _ in 1..=1024 {
        let (send, recv) = chan.to_pair();
        worker.send(Cmd::Cos(750.0, send)).await;
        (_resp, chan) = recv.recv_chan().await;
    }
    let now = Instant::now();
    for _ in 1..=1024*256 {
        let (send, recv) = chan.to_pair();
        worker.send(Cmd::Cos(750.0, send)).await;
        (_resp, chan) = recv.recv_chan().await;
    }
    let elapsed = now.elapsed() / (1024*256);
    println!("Whisk (1-thread): {:?}", elapsed);

    // Tell worker to stop
    worker.stop().await;
    join.recv().await;
}

async fn flume_multi() {
    // Create worker on new thread
    let (worker, tasker) = flume::bounded(0);
    let worker_thread = std::thread::spawn(move || {
        pasts::Executor::default()
            .spawn(Box::pin(async move { worker_flume(tasker).await }))
    });

    let (send, recv) = Channel::pair();
    worker.send_async(Cmd::Cos(750.0, send)).await.unwrap();
    let (mut _resp, mut chan) = recv.recv_chan().await;
    for _ in 1..=1024 {
        let (send, recv) = chan.to_pair();
        worker.send_async(Cmd::Cos(750.0, send)).await.unwrap();
        (_resp, chan) = recv.recv_chan().await;
    }
    let now = Instant::now();
    for _ in 1..=1024*256 {
        let (send, recv) = chan.to_pair();
        worker.send_async(Cmd::Cos(750.0, send)).await.unwrap();
        (_resp, chan) = recv.recv_chan().await;
    }
    let elapsed = now.elapsed() / (1024*256);
    println!("Flume (2-thread): {:?}", elapsed);

    // Tell worker to stop
    drop(worker);
    worker_thread.join().unwrap();
}

async fn flume_single(executor: &Executor) {
    // Create worker on new thread
    let (task, join) = Channel::pair();
    let (worker, tasker) = flume::bounded(0);
    executor.spawn(Box::pin(async move {
        worker_flume(tasker).await;
        task.send(()).await;
    }));

    let (send, recv) = Channel::pair();
    worker.send_async(Cmd::Cos(750.0, send)).await.unwrap();
    let (mut _resp, mut chan) = recv.recv_chan().await;
    for _ in 1..=1024 {
        let (send, recv) = chan.to_pair();
        worker.send_async(Cmd::Cos(750.0, send)).await.unwrap();
        (_resp, chan) = recv.recv_chan().await;
    }
    let now = Instant::now();
    for _ in 1..=1024*256 {
        let (send, recv) = chan.to_pair();
        worker.send_async(Cmd::Cos(750.0, send)).await.unwrap();
        (_resp, chan) = recv.recv_chan().await;
    }
    let elapsed = now.elapsed() / (1024*256);
    println!("Flume (1-thread): {:?}", elapsed);

    // Tell worker to stop
    drop(worker);
    join.recv().await;
}

async fn dyn_lib() {
    let dl_api =
        DlApi::new(CStr::from_bytes_with_nul(b"libm.so.6\0").unwrap()).unwrap();
    let cosf: unsafe extern "C" fn(f32) -> f32 = unsafe {
        std::mem::transmute(
            dl_api
                .get(CStr::from_bytes_with_nul(b"cosf\0").unwrap())
                .unwrap(),
        )
    };

    for _ in 1..=1024 {
        unsafe {
            core::convert::identity(cosf(750.0));
        }
    }
    let now = Instant::now();
    for _ in 1..=1024*256 {
        unsafe {
            core::convert::identity(cosf(750.0));
        }
    }
    let elapsed = now.elapsed() / (1024*256);
    println!("Dynamic library: {:?}", elapsed);
}

async fn tasker(executor: Executor) {
    dyn_lib().await;
    tasker_multi().await;
    flume_multi().await;
    tasker_single(&executor).await;
    flume_single(&executor).await;
}

// Call into executor of your choice
fn main() {
    let executor = pasts::Executor::default();
    executor.spawn(Box::pin(tasker(executor.clone())))
}
