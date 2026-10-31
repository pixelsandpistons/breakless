//! Twitch's HTTP endpoints, guest IRC, emote providers and an exit proxy on loopback.
//!
//! Channels start live with five stream segments. Their five-segment window advances every two
//! seconds; tests can speed the clock up or use a manual script. Segment durations stay two seconds.

mod chat;
mod emotes;
mod http;
mod playlist;
mod proxy;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::AbortHandle;
use tokio::time::Instant;

pub use proxy::Proxy;

const WINDOW: usize = 5;

/// An HTTP request received by the fake, including failures scripted before a response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// HTTP method, such as `GET` or `POST`.
    pub method: String,
    /// Request URI, including its query string.
    pub uri: String,
    /// The `Via` header, or `None` for a direct request.
    pub via: Option<String>,
}

struct AdvertBreak {
    start: usize,
    end: usize,
    roll: String,
}

struct Script {
    ads: Vec<bool>,
    pos: usize,
    offline: bool,
    name: String,
    viewers: u64,
    country: String,
    interval: Duration,
    updated: Instant,
    legacy: bool,
    breaks: Vec<AdvertBreak>,
    drop_next: usize,
    status: Option<(u16, usize)>,
    requests: Vec<Request>,
}

impl Script {
    fn new(login: &str) -> Self {
        Self {
            ads: Vec::new(),
            pos: WINDOW,
            offline: false,
            name: login.to_uppercase(),
            viewers: 100,
            country: "CA".to_string(),
            interval: Duration::from_secs(2),
            updated: Instant::now(),
            legacy: false,
            breaks: Vec::new(),
            drop_next: 0,
            status: None,
            requests: Vec::new(),
        }
    }

    fn sync(&mut self) {
        if !self.interval.is_zero() {
            let now = Instant::now();
            let elapsed = now.duration_since(self.updated);
            let ticks = elapsed.as_nanos() / self.interval.as_nanos();
            let ticks = usize::try_from(ticks).unwrap_or(usize::MAX);
            self.pos = self.pos.saturating_add(ticks);
            if ticks > 0 {
                // Retain the partial tick so reads do not slow the clock down.
                let remainder = elapsed.as_nanos() % self.interval.as_nanos();
                self.updated =
                    now - Duration::from_nanos(u64::try_from(remainder).unwrap_or(u64::MAX));
            }
        }
    }

    fn is_ad(&self, i: usize) -> bool {
        self.ads.get(i).copied().unwrap_or(false)
            || self.breaks.iter().any(|b| b.start <= i && i < b.end)
    }
}

#[derive(Default)]
struct Inner {
    channels: BTreeMap<String, Script>,
    fail_media_next: usize,
    requests: Vec<String>,
    irc_received: Vec<String>,
    emote_status: Vec<(String, u16)>,
    emote_requests: Vec<String>,
}

struct Servers(Vec<AbortHandle>);

impl Drop for Servers {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

/// Three HTTP listeners and a plain TCP IRC server, kept alive until the last handle is dropped.
#[derive(Clone)]
pub struct FakeTwitch {
    /// GraphQL and emote-provider listener.
    pub gql: SocketAddr,
    /// Master-playlist listener.
    pub usher: SocketAddr,
    /// Media-playlist and segment listener.
    pub cdn: SocketAddr,
    irc: SocketAddr,
    inner: Arc<Mutex<Inner>>,
    say: broadcast::Sender<String>,
    servers: Option<Arc<Servers>>,
}

/// Controls one channel without exposing the fake's shared state.
#[derive(Clone)]
pub struct ChannelHandle {
    fake: FakeTwitch,
    login: String,
}

#[derive(Clone, Copy)]
pub(crate) enum Fault {
    Pass,
    Drop,
    Status(u16),
}

impl FakeTwitch {
    /// Starts independent HTTP listeners and the IRC listener on unused loopback ports.
    ///
    /// # Panics
    /// Panics if a loopback listener cannot be bound or its address cannot be read.
    pub async fn start() -> Self {
        let gql = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake GraphQL");
        let usher = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake usher");
        let cdn = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake CDN");
        let irc = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake IRC");
        let (say, _) = broadcast::channel(64);
        let mut fake = Self {
            gql: gql.local_addr().expect("fake GraphQL address"),
            usher: usher.local_addr().expect("fake usher address"),
            cdn: cdn.local_addr().expect("fake CDN address"),
            irc: irc.local_addr().expect("fake IRC address"),
            inner: Arc::new(Mutex::new(Inner::default())),
            say,
            servers: None,
        };
        let mut tasks = Vec::new();
        for listener in [gql, usher, cdn] {
            tasks.push(http::serve(listener, fake.clone()).abort_handle());
        }
        tasks.push(chat::serve(irc, fake.clone()).abort_handle());
        fake.servers = Some(Arc::new(Servers(tasks)));
        fake
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Creates a live channel if absent, or returns another handle to its existing state.
    pub fn channel(&self, login: &str) -> ChannelHandle {
        let login = login.to_lowercase();
        self.inner()
            .channels
            .entry(login.clone())
            .or_insert_with(|| Script::new(&login));
        ChannelHandle {
            fake: self.clone(),
            login,
        }
    }

    /// GraphQL endpoint for the service's configuration.
    pub fn gql_url(&self) -> String {
        format!("http://{}/gql", self.gql)
    }

    /// Master endpoint template, with the service's literal `{login}` substitution.
    pub fn usher_url(&self) -> String {
        format!("http://{}/api/channel/hls/{{login}}.m3u8", self.usher)
    }

    /// Plain TCP guest chat endpoint for the service's configuration.
    pub fn irc_url(&self) -> String {
        format!("tcp://{}", self.irc)
    }

    /// Base replacing `https://` in the emote-provider addresses.
    pub fn emotes_url(&self) -> String {
        format!("http://{}/emotes", self.gql)
    }

    /// Starts an HTTP exit proxy which forwards only to these fake endpoints.
    ///
    /// # Panics
    /// Panics if the proxy listener cannot be bound or its address cannot be read.
    pub async fn proxy(&self) -> Proxy {
        proxy::start([self.gql, self.usher, self.cdn])
            .await
            .expect("start fake exit proxy")
    }

    /// Marks a channel offline, creating it if necessary.
    pub fn offline(&self, login: &str) {
        self.channel(login).offline();
    }

    /// Moves a channel's window by `n` segments and resets its clock's partial tick.
    pub fn advance(&self, login: &str, n: usize) {
        self.channel(login).advance(n);
    }

    /// Makes the next `n` media requests across channels answer 503.
    pub fn fail_media_next(&self, n: usize) {
        self.inner().fail_media_next = n;
    }

    /// Request labels in receipt order; GraphQL and playlist labels retain the original form.
    pub fn requests(&self) -> Vec<String> {
        self.inner().requests.clone()
    }

    /// Broadcasts one IRC line to every connected chat client.
    pub fn irc_say(&self, line: &str) {
        let _ = self.say.send(line.to_string());
    }

    /// IRC lines sent by clients, in receipt order, without line endings.
    pub fn irc_received(&self) -> Vec<String> {
        self.inner().irc_received.clone()
    }

    /// Makes emote addresses containing `piece` answer `status` from now on.
    pub fn emote_status(&self, piece: &str, status: u16) {
        self.inner().emote_status.push((piece.to_string(), status));
    }

    /// Emote addresses requested, relative to the emote base, in receipt order.
    pub fn emote_requests(&self) -> Vec<String> {
        self.inner().emote_requests.clone()
    }

    pub(crate) fn record(
        &self,
        method: &str,
        uri: &str,
        via: Option<&str>,
        logins: &[String],
        label: String,
    ) -> Fault {
        let mut inner = self.inner();
        let is_media = label.starts_with("media ");
        inner.requests.push(label);
        let request = Request {
            method: method.to_string(),
            uri: uri.to_string(),
            via: via.map(str::to_string),
        };
        for login in logins {
            if let Some(channel) = inner.channels.get_mut(login) {
                channel.requests.push(request.clone());
            }
        }
        for login in logins {
            if let Some(channel) = inner.channels.get_mut(login) {
                if channel.drop_next > 0 {
                    channel.drop_next -= 1;
                    return Fault::Drop;
                }
                if let Some((code, remaining)) = &mut channel.status
                    && *remaining > 0
                {
                    *remaining -= 1;
                    return Fault::Status(*code);
                }
            }
        }
        if is_media && inner.fail_media_next > 0 {
            inner.fail_media_next -= 1;
            return Fault::Status(503);
        }
        Fault::Pass
    }

    pub(crate) fn gql(&self, req: &Value) -> Value {
        let query = req["query"].as_str().unwrap_or("");
        let inner = self.inner();
        if query.contains("PlaybackAccessToken") {
            let login = req["variables"]["login"]
                .as_str()
                .unwrap_or("")
                .to_lowercase();
            let token = inner
                .channels
                .get(&login)
                .map(|_| json!({"value": format!("tok-{login}"), "signature": "sig"}));
            return json!({"data": {"streamPlaybackAccessToken": token}});
        }
        if query.contains("users(logins") {
            let users: Vec<Value> = req["variables"]["logins"].as_array().into_iter().flatten().filter_map(Value::as_str)
                .map(|l| match inner.channels.get(l) {
                    Some(c) if !c.offline => json!({"login": l, "displayName": c.name, "stream": {"title": format!("{l} live"), "viewersCount": c.viewers, "game": {"displayName": "Games"}}}),
                    _ => json!({"login": l, "displayName": l.to_uppercase(), "stream": null}),
                }).collect();
            return json!({"data": {"users": users}});
        }
        if query.contains("streams(first") {
            let edges: Vec<Value> = inner.channels.iter().filter(|(_, c)| !c.offline)
                .map(|(l,c)| json!({"node": {"title": format!("{l} live"), "viewersCount": c.viewers, "broadcaster": {"login": l, "displayName": c.name}, "game": {"displayName": "Games"}}})).collect();
            return json!({"data": {"streams": {"edges": edges}}});
        }
        json!({"errors": [{"message": "unknown query"}]})
    }
}

impl ChannelHandle {
    fn change(&self, f: impl FnOnce(&mut Script)) {
        if let Some(channel) = self.fake.inner().channels.get_mut(&self.login) {
            f(channel);
        }
    }

    /// Uses the old fixture's exact manual script and synthetic `cdn.example` segment addresses.
    /// `pos` segments have arrived; flags past the script's end are clean stream segments.
    pub fn script(&self, ads: &[bool], pos: usize) {
        self.change(|c| {
            c.ads = ads.to_vec();
            c.pos = pos;
            c.interval = Duration::ZERO;
            c.updated = Instant::now();
            c.legacy = true;
            c.breaks.clear();
        });
    }

    /// Sets elapsed wall time per new segment. Zero selects manual advancement only.
    /// Each rendered segment still lasts two seconds.
    pub fn segment_interval(&self, interval: Duration) {
        self.change(|c| {
            c.sync();
            c.interval = interval;
            c.updated = Instant::now();
        });
    }

    /// Moves the window on by `n` segments and starts a fresh clock tick.
    pub fn advance(&self, n: usize) {
        self.change(|c| {
            c.sync();
            c.pos = c.pos.saturating_add(n);
            c.updated = Instant::now();
        });
    }

    /// Makes usher and media answer 404 and excludes this channel from live lists.
    pub fn offline(&self) {
        self.change(|c| {
            c.offline = true;
            c.viewers = 0;
        });
    }

    /// Starts an isolated advert window with `segments` two-second segments and the given roll.
    /// The first advert is available immediately. After the remaining adverts arrive, clean
    /// segments resume with a discontinuity; old clean or advert segments do not fill that window.
    /// Zero segments leaves the stream unchanged.
    pub fn advert_break(&self, segments: usize, roll: &str) {
        if segments == 0 {
            return;
        }
        self.change(|c| {
            c.sync();
            let start = c.pos;
            c.breaks.push(AdvertBreak {
                start,
                end: start.saturating_add(segments),
                roll: roll.to_string(),
            });
            c.pos = start.saturating_add(1);
            c.updated = Instant::now();
        });
    }

    /// Closes the next `n` channel requests without a response, before scripted statuses.
    pub fn drop_next(&self, n: usize) {
        self.change(|c| c.drop_next = n);
    }

    /// Answers the next `n` channel requests with `code`, after any scripted connection drops.
    /// Invalid HTTP codes are served as 500.
    pub fn status(&self, code: u16, n: usize) {
        self.change(|c| c.status = Some((code, n)));
    }

    /// Sets `USER-COUNTRY` in this channel's master playlist.
    pub fn country(&self, cc: &str) {
        self.change(|c| c.country = cc.to_string());
    }

    /// Channel requests in receipt order, including scripted failures and proxy `Via` headers.
    pub fn requests(&self) -> Vec<Request> {
        self.fake
            .inner()
            .channels
            .get(&self.login)
            .map_or_else(Vec::new, |c| c.requests.clone())
    }
}
