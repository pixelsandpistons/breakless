use crate::FakeTwitch;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

pub(crate) fn serve(listener: TcpListener, fake: FakeTwitch) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut clients = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((socket, _)) = accepted else { break; };
                    clients.spawn(fake.clone().irc_client(socket));
                }
                _ = clients.join_next(), if !clients.is_empty() => {}
            }
        }
    })
}

impl FakeTwitch {
    async fn irc_client(self, socket: tokio::net::TcpStream) {
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();
        let mut say = self.say.subscribe();
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { break };
                    self.inner().irc_received.push(line.clone());
                    if let Some(channel) = line.strip_prefix("JOIN #") {
                        let answer = format!(
                            ":tmi.twitch.tv 001 justinfan1 :Welcome, GLHF!\r\n@emote-only=0;room-id=71092938;slow=0 :tmi.twitch.tv ROOMSTATE #{channel}\r\n"
                        );
                        if writer.write_all(answer.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                }
                said = say.recv() => {
                    let Ok(said) = said else { break };
                    if writer.write_all(format!("{said}\r\n").as_bytes()).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}
