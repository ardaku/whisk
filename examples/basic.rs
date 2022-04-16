use whisk::Messenger;

enum Msg {
    /// Messenger has finished initialization
    Ready,
}

enum Cmd {
    /// Tell messenger to quit
    Exit,
}

async fn messenger_task(mut messenger: Messenger<Cmd, Msg>) {
    // Some work
    println!("Doing initialization work....");

    // Receive command from commander
    while let Some(command) = messenger.next().unwrap().await {
        match command.get() {
            Cmd::Exit => {
                println!("Messenger received exit, shutting down....");
                command.close(messenger);
                return;
            }
        }
    }

    unreachable!()
}

async fn commander_task() {
    let (mut commander, messenger) = whisk::channel(Msg::Ready).await;
    let messenger = messenger_task(messenger);

    // Start task on another thread
    let messenger = std::thread::spawn(|| pasts::block_on(messenger));

    // Wait for Ready message, and respond with Exit command
    println!("Waiting messages....");
    while let Some(message) = commander.next().unwrap().await {
        match message.get() {
            Msg::Ready => {
                println!("Received ready, telling messenger to exit....");
                message.respond(Cmd::Exit)
            }
        }
    }

    // Wait for messenger to close down
    messenger.join().unwrap();
    println!("Messenger has exited, now too shall the commander");
}

// Call into executor of your choice
fn main() {
    pasts::block_on(commander_task())
}
