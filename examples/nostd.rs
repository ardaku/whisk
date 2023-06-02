//! Use whisk on no-std, specifically targeting x86_64-unknown-linux-gnu (may
//! work on others).  Requires nightly for `eh_personality` lang item (tested
//! with `rustc 1.72.0-nightly (871b59520 2023-05-31)`).
//!
//! ```shell
//! cargo +nightly run --release --example nostd --features=pasts
//! ```

#![no_std]
#![no_main]
#![feature(lang_items)]

extern crate alloc;

#[global_allocator]
static _GLOBAL_ALLOCATOR: rlsf::SmallGlobalTlsf = rlsf::SmallGlobalTlsf::new();

#[lang = "eh_personality"]
extern "C" fn _eh_personality() {}

#[no_mangle]
extern "C" fn _Unwind_Resume() {}

//// End no-std specific boilerplate ////

use alloc::string::ToString;

use pasts::{prelude::*, Executor};
use whisk::Channel;

fn println(string: impl ToString) {
    #[link(name = "c")]
    extern "C" {
        fn puts(s: *const core::ffi::c_char) -> core::ffi::c_int;
    }

    let message = alloc::ffi::CString::new(string.to_string()).unwrap();

    unsafe { puts(message.as_ptr()) };
}

#[panic_handler]
fn yeet(panic: &::core::panic::PanicInfo<'_>) -> ! {
    println(panic);

    loop {}
}

#[no_mangle]
pub extern "C" fn main() -> ! {
    // Not an efficient executor for no-std, but it works
    let executor = Executor::default();

    executor.clone().block_on(async move {
        println("Hello from a future");

        let channel = Channel::new();
        let mut channel_clone = channel.clone();

        executor.spawn_boxed(async move {
            channel_clone.next().await;
            println("Received message");
        });

        executor.spawn_boxed(async move {
            println("Sending message");
            channel.send(()).await;
        });

        println("End of primary async block");
    });

    println("Exit pasts executor");

    loop {}
}
