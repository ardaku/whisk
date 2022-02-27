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
    let command = (&mut messenger).await.unwrap();
    match command.get() {
        Cmd::Exit => {
            println!("Shutting down messenger....");
            command.close(messenger)
        },
    }
}

async fn commander_task() {
    let (mut commander, messenger) = whisk::channel(Msg::Ready).await;
    let messenger = messenger_task(messenger);

    // Start task on another thread
    std::thread::spawn(|| pasts::block_on(messenger));

    // wait for Ready message, and respond with Exit command
    println!("Waiting messages....");
    while let Some(message) = (&mut commander).await {
        match message.get() {
            Msg::Ready => {
                println!("Received ready, telling messenger to exit....");
                message.respond(Cmd::Exit)
            },
        }
    }
    println!("Messenger has exited, now too shall the commander");
}

fn main() {
    pasts::block_on(commander_task())
}
