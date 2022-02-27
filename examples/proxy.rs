use whisk::{Commander, Messenger};

pub enum Msg {
    /// Messenger has finished initialization
    Ready,
    /// Solved the addition
    Output(u32),
}

pub enum Cmd {
    /// Tell messenger to quit
    Exit,
    /// Request addition
    Add(u32, u32),
}

pub struct Proxy {
    commander: Commander<Cmd, Msg>,
    pub message: Option<whisk::Message<Cmd, Msg>>,
}

impl Proxy {
    #[inline(always)]
    pub async fn do_addition(&mut self, a: u32, b: u32) -> u32 {
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
}

async fn messenger_task(mut messenger: Messenger<Cmd, Msg>) {
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

pub async fn commander_task() -> Proxy {
    let (mut commander, messenger) = whisk::channel(Msg::Ready).await;
    let messenger = messenger_task(messenger);

    // Start task on another thread
    std::thread::spawn(|| pasts::block_on(messenger));

    // Wait for Ready message, and respond with Exit command
    while let Some(message) = (&mut commander).await {
        match message.get() {
            Msg::Ready => {
                return Proxy {
                    message: Some(message),
                    commander,
                }
            }
            _ => unreachable!(),
        }
    }

    unreachable!()
}

fn main() {
    pasts::block_on(async {
        let mut proxy = commander_task().await;

        proxy.message.take().unwrap().respond(Cmd::Add(43, 400));
        proxy.message = (&mut proxy.commander).await;
        match proxy.message.as_ref().unwrap().get() {
            Msg::Output(x) => println!("{x}"),
            _ => unreachable!(),
        }
        proxy.message.take().unwrap().respond(Cmd::Exit);
        proxy.message = (&mut proxy.commander).await;
    })
}
