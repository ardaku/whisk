use std::{ffi::CStr, time::Instant};

use dl_api::manual::DlApi;
use pasts::{prelude::*, Executor};
use whisk::Channel;

enum Cmd {
    /// Tell messenger to get cosine
    Cos(f32, Channel<f32>),
}

async fn worker(channel: Channel<Option<Cmd>>) {
    while let Some(command) = channel.recv().await {
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
    let chan = Channel::new();
    let tasker = chan.clone();
    let worker_thread = std::thread::spawn(move || {
        Executor::default().block_on(async move { worker(tasker).await })
    });
    let worker = chan;

    let channel = Channel::new();
    for _ in 1..=1024 {
        worker.send(Some(Cmd::Cos(750.0, channel.clone()))).await;
        channel.recv().await;
    }
    let now = Instant::now();
    for _ in 1..=1024 * 256 {
        worker.send(Some(Cmd::Cos(750.0, channel.clone()))).await;
        channel.recv().await;
    }
    let elapsed = now.elapsed() / (1024 * 256);
    println!("Whisk (2-thread): {:?}", elapsed);

    // Tell worker to stop
    worker.send(None).await;
    worker_thread.join().unwrap();
}

async fn tasker_single(executor: &Executor) {
    // Create worker on new thread
    let chan = Channel::new();
    let join = Channel::new();
    executor.spawn({
        let tasker = chan.clone();
        let join = join.clone();
        async move {
            worker(tasker).await;
            join.send(()).await;
        }
    });
    let worker = chan;

    let channel = Channel::new();
    for _ in 1..=1024 {
        worker.send(Some(Cmd::Cos(750.0, channel.clone()))).await;
        channel.recv().await;
    }
    let now = Instant::now();
    for _ in 1..=1024 * 256 {
        worker.send(Some(Cmd::Cos(750.0, channel.clone()))).await;
        channel.recv().await;
    }
    let elapsed = now.elapsed() / (1024 * 256);
    println!("Whisk (1-thread): {:?}", elapsed);

    // Tell worker to stop
    worker.send(None).await;
    join.recv().await;
}

async fn flume_multi() {
    // Create worker on new thread
    let (worker, tasker) = flume::bounded(1);
    let worker_thread = std::thread::spawn(move || {
        Executor::default().block_on(async move { worker_flume(tasker).await })
    });

    let channel = Channel::new();
    for _ in 1..=1024 {
        worker
            .send_async(Cmd::Cos(750.0, channel.clone()))
            .await
            .unwrap();
        channel.recv().await;
    }
    let now = Instant::now();
    for _ in 1..=1024 * 256 {
        worker
            .send_async(Cmd::Cos(750.0, channel.clone()))
            .await
            .unwrap();
        channel.recv().await;
    }
    let elapsed = now.elapsed() / (1024 * 256);
    println!("Flume (2-thread): {:?}", elapsed);

    // Tell worker to stop
    drop(worker);
    worker_thread.join().unwrap();
}

async fn flume_single(executor: &Executor) {
    // Create worker on new thread
    let join = Channel::new();
    let (worker, tasker) = flume::bounded(1);
    executor.spawn({
        let join = join.clone();
        async move {
            worker_flume(tasker).await;
            join.send(()).await;
        }
    });

    let channel = Channel::new();
    for _ in 1..=1024 {
        worker
            .send_async(Cmd::Cos(750.0, channel.clone()))
            .await
            .unwrap();
        channel.recv().await;
    }
    let now = Instant::now();
    for _ in 1..=1024 * 256 {
        worker
            .send_async(Cmd::Cos(750.0, channel.clone()))
            .await
            .unwrap();
        channel.recv().await;
    }
    let elapsed = now.elapsed() / (1024 * 256);
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
    for _ in 1..=1024 * 256 {
        unsafe {
            core::convert::identity(cosf(750.0));
        }
    }
    let elapsed = now.elapsed() / (1024 * 256);
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
    let executor = Executor::default();
    executor.clone().block_on(tasker(executor));
}
