use core::future::Future;

use pasts::Executor;
use whisk::Channel;

fn check_send_and_exec(f: impl Future<Output = ()> + Send + 'static) {
    Executor::default().spawn(f)
}

#[test]
fn weak() {
    check_send_and_exec(async {
        let channel = Channel::new();
        let weak = channel.downgrade();
        let result = weak.try_send(()).await;

        assert!(result.is_ok());

        let result = weak.try_recv().await;

        assert!(result.is_ok());
    });
}
