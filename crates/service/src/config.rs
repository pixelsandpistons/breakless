//! What the service is told at start: addresses, exits, folders.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Args;

/// Settings for `breakless serve`. Every flag has an environment variable so a compose file can
/// set it; the `TWITCH_*` names are the ones the Python prototype used and keep working.
#[derive(Args, Debug, Clone)]
pub struct Config {
    /// Address to listen on. Inside a container `0.0.0.0:8080`; on a shared host `127.0.0.1:8080`.
    #[arg(long, default_value = "0.0.0.0:8080", env = "BREAKLESS_LISTEN")]
    pub listen: SocketAddr,

    /// HTTP proxies in countries without Twitch adverts, comma separated, tried in order.
    /// Empty: ask Twitch straight out and only cut the adverts (the picture then pauses for each break).
    #[arg(long, env = "TWITCH_PROXY", default_value = "", value_delimiter = ',')]
    pub proxy: Vec<String>,

    /// The host:port people open the page at; Twitch's chat frame refuses a mismatch.
    #[arg(long, env = "TWITCH_HOST", default_value = "")]
    pub host: String,

    /// Where `follows.json` and `adverts.jsonl` live.
    #[arg(long, env = "TWITCH_DATA", default_value = "/data")]
    pub data: PathBuf,

    /// The page's files (`index.html`, `app.js`, ...).
    #[arg(long, env = "BREAKLESS_WEB", default_value = "/web")]
    pub web: PathBuf,

    /// Twitch's GraphQL endpoint. Only tests change it.
    #[arg(
        long,
        env = "BREAKLESS_GQL_URL",
        default_value = "https://gql.twitch.tv/gql",
        hide = true
    )]
    pub gql_url: String,

    /// Where live playlists come from; `{login}` is replaced. Only tests change it.
    #[arg(
        long,
        env = "BREAKLESS_USHER_URL",
        default_value = "https://usher.ttvnw.net/api/channel/hls/{login}.m3u8",
        hide = true
    )]
    pub usher_url: String,

    /// Twitch's chat server, `tls://host:port`; `tcp://` for a plain socket. Only tests change it.
    #[arg(
        long,
        env = "BREAKLESS_IRC_URL",
        default_value = "tls://irc.chat.twitch.tv:6697",
        hide = true
    )]
    pub irc_url: String,

    /// A base that replaces `https://` in the emote providers' addresses. Only tests set it.
    #[arg(long, env = "BREAKLESS_EMOTES_URL", default_value = "", hide = true)]
    pub emotes_url: String,
}

impl Config {
    /// The proxies with empty entries dropped.
    pub fn proxies(&self) -> Vec<String> {
        self.proxy
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect()
    }
}
