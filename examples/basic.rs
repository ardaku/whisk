use whisk::Messenger;

enum Msg {
    /// Messenger has finished initialization
    Ready,
    /// Result of addition
    Response(u32),
}

enum Cmd {
    /// Tell messenger to add
    Add(u32, u32),
    /// Tell messenger to quit
    Exit,
}

async fn messenger_task(mut messenger: Messenger<Cmd, Msg>) {
    // Send ready and receive command from commander
    println!("Messenger sending ready");
    messenger.start().await;

    for command in &mut messenger {
        let responder = match command.get() {
            Cmd::Add(a, b) => {
                println!("Messenger received add, sending response");
                let result = *a + *b;
                command.respond(Msg::Response(result))
            }
            Cmd::Exit => {
                println!("Messenger received exit, shutting down…");
                return;
            }
        };
        responder.await
    }

    unreachable!()
}

async fn commander_task() {
    let (mut commander, messenger) = whisk::channel(Msg::Ready).await;

    // Start messenger task on another thread
    let messenger = messenger_task(messenger);
    let messenger = std::thread::spawn(|| pasts::block_on(messenger));

    // Wait for Ready message, and respond with Exit command
    println!("Commander waiting ready message…");
    commander.start().await;
    for message in &mut commander {
        let responder = match message.get() {
            Msg::Ready => {
                println!("Commander received ready, sending add command…");
                message.respond(Cmd::Add(43, 400))
            }
            Msg::Response(value) => {
                assert_eq!(*value, 443);
                println!("Commander received response, commanding exit…");
                message.respond(Cmd::Exit)
            }
        };
        responder.await
    }

    println!("Commander disconnected");
    messenger.join().unwrap();
    println!("Messenger thread joined");
}

// Call into executor of your choice
fn main() {
    pasts::block_on(commander_task())
}
