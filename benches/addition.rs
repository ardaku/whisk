use std::ffi::CStr;

use criterion::{
    async_executor::FuturesExecutor, criterion_group, criterion_main,
    BenchmarkId, Criterion,
};
use dl_api::manual::DlApi;

mod ffi {
    #[no_mangle]
    #[inline(never)]
    extern "C" fn extern_addition(a: u32, b: u32) -> u32 {
        a + b
    }
}

extern "C" {
    fn extern_addition(a: u32, b: u32) -> u32;
}

#[inline(never)]
fn do_addition(a: u32, b: u32) -> u32 {
    a + b
}

mod channel {
    use whisk::{Commander, Messenger};

    pub(super) enum Msg {
        /// Messenger has finished initialization
        Ready,
        /// Solved the addition
        Output(u32),
    }

    pub(super) enum Cmd {
        /// Tell messenger to quit
        Exit,
        /// Request addition
        Add(u32, u32),
    }

    pub(super) struct Proxy {
        commander: Commander<Cmd, Msg>,
        pub(super) message: Option<whisk::Message<Cmd, Msg>>,
    }

    impl Proxy {
        #[inline(always)]
        pub(super) async fn do_addition(&mut self, a: u32, b: u32) -> u32 {
            let message = self.message.take().unwrap();
            message.respond(Cmd::Add(a, b));
            let message = (&mut self.commander).await;
            match message {
                Some(msg) => match *msg.get() {
                    Msg::Output(v) => {
                        self.message = Some(msg);
                        v
                    }
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            }
        }

        pub(super) async fn close(&mut self) {
            let message = self.message.take().unwrap();
            message.respond(Cmd::Exit);
            let message = (&mut self.commander).await;
            match message {
                None => {}
                _ => unreachable!(),
            }
        }
    }

    pub(super) async fn messenger_task(mut messenger: Messenger<Cmd, Msg>) {
        // Receive command from commander
        while let Some(command) = (&mut messenger).await {
            match *command.get() {
                Cmd::Add(a, b) => command.respond(Msg::Output(a + b)),
                Cmd::Exit => {
                    command.close(messenger);
                    return;
                }
            }
        }
        unreachable!()
    }

    pub(super) async fn commander_task(
        mut commander: Commander<Cmd, Msg>,
    ) -> Proxy {
        // Wait for Ready message, and respond with Exit command
        while let Some(message) = (&mut commander).await {
            match message.get() {
                Msg::Ready => {
                    return Proxy {
                        message: Some(message),
                        commander,
                    };
                }
                _ => unreachable!(),
            }
        }

        unreachable!()
    }
}

struct Args<'a>(u32, u32, &'a mut channel::Proxy);

impl std::fmt::Display for Args<'_> {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> Result<(), std::fmt::Error> {
        write!(f, "Args")
    }
}

pub fn criterion_benchmark(c: &mut Criterion) {
    let dl_api =
        DlApi::new(CStr::from_bytes_with_nul(b"libaddition.so\0").unwrap())
            .unwrap();
    let ffi_addition: unsafe extern "C" fn(u32, u32) -> u32 = unsafe {
        std::mem::transmute(
            dl_api
                .get(CStr::from_bytes_with_nul(b"ffi_addition\0").unwrap())
                .unwrap(),
        )
    };
    let mut proxy = futures::executor::block_on(async {
        let (commander, messenger) = whisk::channel(channel::Msg::Ready).await;
        let messenger = channel::messenger_task(messenger);
        std::thread::spawn(|| pasts::block_on(messenger));
        channel::commander_task(commander).await
    });

    let (mut proxy_single, mut task) = futures::executor::block_on(async {
        let (commander, messenger) = whisk::channel(channel::Msg::Ready).await;
        let mut messenger = Box::pin(channel::messenger_task(messenger));
        let _ = futures::poll!(&mut messenger);
        (channel::commander_task(commander).await, messenger)
    });

    let (msgr_to_cmdr, cmdr_from_msgr) = flume::bounded(1);
    let (cmdr_to_msgr, msgr_from_cmdr) = flume::bounded(1);
    let flume_thread = std::thread::spawn(move || pasts::block_on(async move {
        while let Ok(command) = msgr_from_cmdr.recv_async().await {
            match command {
                channel::Cmd::Add(a, b) => if msgr_to_cmdr.send_async(a + b).await.is_err() {
                    break
                },
                channel::Cmd::Exit => break,
            }
        }
    }));
    let (msgr_to_cmdr_task, cmdr_from_msgr_task) = flume::bounded(1);
    let (cmdr_to_msgr_task, msgr_from_cmdr_task) = flume::bounded(1);
    let mut flume_task = Box::pin(async move {
        while let Ok(command) = msgr_from_cmdr_task.recv_async().await {
            match command {
                channel::Cmd::Add(a, b) => if msgr_to_cmdr_task.send_async(a + b).await.is_err() {
                    break
                },
                channel::Cmd::Exit => break,
            }
        }
    });

    let inputs = (453, 198_231_014);

    c.bench_with_input(
        BenchmarkId::new("function", "call"),
        &inputs,
        |z, args| {
            // Insert a call to `to_async` to convert the bencher to async mode.
            // The timing loops are the same as with the normal bencher.
            z.to_async(FuturesExecutor)
                .iter(|| async { do_addition(args.0, args.1) });
        },
    );
    c.bench_with_input(
        BenchmarkId::new("extern", "call"),
        &inputs,
        |z, args| {
            // Insert a call to `to_async` to convert the bencher to async mode.
            // The timing loops are the same as with the normal bencher.
            z.to_async(FuturesExecutor)
                .iter(|| async { unsafe { extern_addition(args.0, args.1) } });
        },
    );
    c.bench_with_input(BenchmarkId::new("ffi", "call"), &inputs, |z, args| {
        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        z.to_async(FuturesExecutor)
            .iter(|| async { unsafe { ffi_addition(args.0, args.1) } });
    });

    let args: *mut _ = &mut (453, 198_231_014, &mut proxy);
    let argz: *mut _ = &mut (453, 198_231_014, &mut proxy_single, &mut task);

    c.bench_with_input(BenchmarkId::new("whisk", "call"), &argz, |z, args| {
        use futures::FutureExt;

        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        z.to_async(FuturesExecutor).iter(|| async {
            let (a, b, f, g) = unsafe { &mut **args };

            let f = async { f.do_addition(*a, *b).await };
            futures::pin_mut!(f);
            let mut f = f.fuse();
            let mut g = g.fuse();

            let _ = loop {
                futures::select! {
                    a = f => break a,
                    _ = g => {},
                };
            };
        });
    });

    c.bench_with_input(
        BenchmarkId::new("whisk", "threads"),
        &args,
        |z, args| {
            // Insert a call to `to_async` to convert the bencher to async mode.
            // The timing loops are the same as with the normal bencher.
            z.to_async(FuturesExecutor).iter(|| async {
                let args = unsafe { &mut **args };
                let a = args.0;
                let b = args.1;

                args.2.do_addition(a, b).await
            });
        },
    );

    futures::executor::block_on(unsafe { (*args).2.close() });
    futures::executor::block_on(async {
        use futures::FutureExt;

        let (_, _, f, g) = unsafe { &mut *argz };

        let f = Box::pin(f.close());
        let mut f = f.fuse();
        let mut g = g.fuse();

        let _ = loop {
            futures::select! {
                a = f => break a,
                _ = g => {},
            };
        };
    });

    // Flume comparisons
    let args: *mut _ = &mut (453, 198_231_014, &cmdr_from_msgr_task, &cmdr_to_msgr_task, &mut flume_task);
    c.bench_with_input(BenchmarkId::new("flume", "call"), &args, |z, args| {
        use futures::FutureExt;

        // Insert a call to `to_async` to convert the bencher to async mode.
        // The timing loops are the same as with the normal bencher.
        z.to_async(FuturesExecutor).iter(|| async {
            let (a, b, cmdr_from_msgr, cmdr_to_msgr, g) = unsafe { &mut **args };

            let f = async {
                cmdr_to_msgr.send_async(channel::Cmd::Add(*a, *b)).await.unwrap();
                cmdr_from_msgr.recv_async().await.unwrap()
            };
            futures::pin_mut!(f);
            let mut f = f.fuse();
            let mut g = g.fuse();

            let _ = loop {
                futures::select! {
                    a = f => break a,
                    _ = g => {},
                };
            };
        });
    });
    c.bench_with_input(
        BenchmarkId::new("flume", "threads"),
        &(453, 198_231_014, &cmdr_from_msgr, &cmdr_to_msgr),
        |z, args| {
            // Insert a call to `to_async` to convert the bencher to async mode.
            // The timing loops are the same as with the normal bencher.
            z.to_async(FuturesExecutor).iter(|| async {
                let (a, b, cmdr_from_msgr, cmdr_to_msgr) = *args;

                cmdr_to_msgr.send_async(channel::Cmd::Add(a, b)).await.unwrap();
                cmdr_from_msgr.recv_async().await.unwrap()
            });
        },
    );

    futures::executor::block_on(async move {
        cmdr_to_msgr.send_async(channel::Cmd::Exit).await.unwrap();
        flume_thread.join().unwrap();
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
