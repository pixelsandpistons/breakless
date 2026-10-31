//! Chat rooms and emote lists: the sockets around `breakless_formats::chat` and `::emotes`.
//!
//! One guest connection per channel somebody is reading, dropped a minute after the last reader
//! went away; at most [`MAX_ROOMS`] at once. Readers long-poll: a request for "everything after
//! number N" is held open up to 25 s until there is something. Chat goes straight out from the
//! home line, never through an exit: it carries no adverts.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use breakless_formats::chat::{Action, Log, parse_line, take};
use breakless_formats::emotes::{
    CHANNEL_AGE, GLOBAL_AGE, RETRY_AGE, SOURCES, Scope, Table, merge, provider_name, valid_room_id,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;

use crate::twitch::{random_u64, truncate};

/// Seconds without a reader before a channel's connection is dropped.
pub const IDLE: Duration = Duration::from_secs(60);
/// Channels read at once; the one least recently read is dropped to make room.
pub const MAX_ROOMS: usize = 12;
/// The longest a poll is held open.
pub const MAX_WAIT: Duration = Duration::from_secs(25);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Twitch pings every five minutes or so; silence longer than this is a dead line.
const SILENCE: Duration = Duration::from_secs(330);
const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Where the IRC server is and how to reach it.
#[derive(Debug, Clone)]
pub struct IrcTarget {
    host: String,
    port: u16,
    tls: bool,
}

impl IrcTarget {
    /// `tls://host:port` (Twitch's `irc.chat.twitch.tv:6697`) or `tcp://host:port` for a plain
    /// socket, which only tests use.
    pub fn parse(url: &str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        let tls = match scheme {
            "tls" => true,
            "tcp" => false,
            _ => return None,
        };
        let (host, port) = rest.rsplit_once(':')?;
        Some(Self {
            host: host.to_string(),
            port: port.parse().ok()?,
            tls,
        })
    }
}

#[derive(Default)]
struct Inner {
    log: Log,
    state: &'static str,
    room_id: String,
}

/// One channel's chat: the connection, the last events, and the readers waiting for the next.
pub struct Room {
    /// The channel's name, lower case.
    pub login: String,
    inner: Mutex<Inner>,
    used: Mutex<Instant>,
    seq: watch::Sender<u64>,
    closed: watch::Sender<bool>,
}

impl Room {
    fn new(login: &str) -> Arc<Self> {
        let (seq, _) = watch::channel(0);
        let (closed, _) = watch::channel(false);
        Arc::new(Self {
            login: login.to_string(),
            inner: Mutex::new(Inner {
                state: "connecting",
                ..Inner::default()
            }),
            used: Mutex::new(Instant::now()),
            seq,
            closed,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn touch(&self) {
        *self
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    }

    fn idle(&self) -> Duration {
        self.used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .elapsed()
    }

    /// The channel's Twitch number, empty until the join is confirmed.
    pub fn room_id(&self) -> String {
        self.lock().room_id.clone()
    }

    fn close(&self) {
        let _ = self.closed.send(true);
    }

    /// What one parsed line does to the room; the line to answer with, if any.
    fn handle(&self, raw: &str) -> Result<Option<String>, &'static str> {
        match take(&parse_line(raw)) {
            Action::Reply(line) => Ok(Some(line)),
            Action::Joined { room_id } => {
                let mut i = self.lock();
                i.room_id = room_id;
                i.state = "joined";
                Ok(None)
            }
            Action::Event(ev) => {
                let n = self.lock().log.add(ev);
                let _ = self.seq.send(n);
                Ok(None)
            }
            Action::Reconnect => Err("Twitch asked for a reconnect"),
            Action::Nothing => Ok(None),
        }
    }

    /// One connection, from the join until it fails. The error carries whether any line was
    /// received: a connection that spoke earns a fresh start on the backoff.
    async fn session(&self, target: &IrcTarget) -> Result<(), (String, bool)> {
        let mut heard = false;
        self.talk(target, &mut heard).await.map_err(|e| (e, heard))
    }

    async fn talk(&self, target: &IrcTarget, heard: &mut bool) -> Result<(), String> {
        let connect = tokio::net::TcpStream::connect((target.host.as_str(), target.port));
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| "connect: timed out".to_string())?
            .map_err(|e| format!("connect: {e}"))?;
        let stream: Box<dyn Stream> = if target.tls {
            let config = <rustls::ClientConfig as rustls_platform_verifier::ConfigVerifierExt>::with_platform_verifier()
                .map_err(|e| format!("TLS: {e}"))?;
            let name = rustls::pki_types::ServerName::try_from(target.host.clone())
                .map_err(|e| format!("TLS name: {e}"))?;
            let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
            Box::new(
                connector
                    .connect(name, tcp)
                    .await
                    .map_err(|e| format!("TLS: {e}"))?,
            )
        } else {
            Box::new(tcp)
        };
        let (reader, mut writer) = tokio::io::split(stream);
        let hello = format!(
            "CAP REQ :twitch.tv/tags twitch.tv/commands\r\nNICK justinfan{}\r\nJOIN #{}\r\n",
            10_000 + random_u64() % 990_000,
            self.login
        );
        writer
            .write_all(hello.as_bytes())
            .await
            .map_err(|e| format!("write: {e}"))?;
        let mut lines = BufReader::new(reader).lines();
        let mut closed = self.closed.subscribe();
        loop {
            let next = tokio::select! {
                l = tokio::time::timeout(SILENCE, lines.next_line()) => l,
                _ = closed.changed() => return Ok(()),
            };
            let line = match next {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => return Err("closed by Twitch".to_string()),
                Ok(Err(e)) => return Err(format!("read: {e}")),
                Err(_) => return Err("silent for too long".to_string()),
            };
            *heard = true;
            if let Some(answer) = self.handle(line.trim_end_matches('\r'))? {
                writer
                    .write_all(format!("{answer}\r\n").as_bytes())
                    .await
                    .map_err(|e| format!("write: {e}"))?;
            }
        }
    }

    /// Keeps the room connected until it is closed, backing off after failures.
    async fn run(self: Arc<Self>, target: IrcTarget) {
        let mut wait = Duration::from_secs(1);
        let mut closed = self.closed.subscribe();
        while !*closed.borrow() {
            if let Err((why, heard)) = self.session(&target).await {
                if *closed.borrow() {
                    break;
                }
                if heard {
                    wait = Duration::from_secs(1);
                }
                self.lock().state = "down";
                tracing::info!(
                    "chat {}: {}; again in {}s",
                    self.login,
                    truncate(&why, 80),
                    wait.as_secs()
                );
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    _ = closed.changed() => break,
                }
                wait = (wait * 2).min(BACKOFF_MAX);
                self.lock().state = "connecting";
            }
        }
    }

    fn answer(&self, after: Option<u64>) -> Value {
        let i = self.lock();
        json!({
            "channel": self.login,
            "seq": i.log.seq(),
            "state": i.state,
            "room_id": i.room_id,
            "events": i.log.since(after),
        })
    }

    /// Events numbered above `after` (`None`: the last 60), waiting up to `wait` for the first.
    pub async fn poll(&self, after: Option<u64>, wait: Duration) -> Value {
        self.touch();
        let deadline = Instant::now() + wait.min(MAX_WAIT);
        let mut seq = self.seq.subscribe();
        let mut closed = self.closed.subscribe();
        loop {
            let current = *seq.borrow_and_update();
            // nothing asked for, or a number we never gave (the service restarted): answer now
            let ready = after.is_none_or(|a| a != current);
            if ready || *closed.borrow() {
                break;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            tokio::select! {
                _ = tokio::time::timeout(left, seq.changed()) => {}
                _ = closed.changed() => {}
            }
        }
        self.touch();
        self.answer(after)
    }
}

trait Stream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Stream for T {}

/// The rooms being read.
pub struct Rooms {
    target: IrcTarget,
    rooms: Mutex<BTreeMap<String, Arc<Room>>>,
}

impl Rooms {
    /// Rooms that will connect to `target`.
    pub fn new(target: IrcTarget) -> Self {
        Self {
            target,
            rooms: Mutex::new(BTreeMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Arc<Room>>> {
        self.rooms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The room for a channel, joined on first sight. Must be called on the runtime.
    pub fn room(&self, login: &str) -> Arc<Room> {
        let login = login.to_lowercase();
        let mut rooms = self.lock();
        if let Some(r) = rooms.get(&login) {
            return r.clone();
        }
        if rooms.len() >= MAX_ROOMS
            && let Some(oldest) = rooms
                .values()
                .max_by_key(|r| r.idle())
                .map(|r| r.login.clone())
        {
            if let Some(r) = rooms.remove(&oldest) {
                r.close();
            }
            tracing::info!("chat {oldest}: dropped to make room");
        }
        let room = Room::new(&login);
        tracing::info!("chat {login}: joining");
        tokio::spawn(room.clone().run(self.target.clone()));
        rooms.insert(login, room.clone());
        room
    }

    /// Leaves rooms nobody has read in [`IDLE`].
    pub fn reap(&self) {
        let mut rooms = self.lock();
        rooms.retain(|login, r| {
            let keep = r.idle() <= IDLE;
            if !keep {
                r.close();
                tracing::info!("chat {login}: nobody reading, left");
            }
            keep
        });
    }

    /// The `chat` list in `/api/status`.
    pub fn status(&self) -> Vec<Value> {
        self.lock()
            .values()
            .map(|r| {
                let i = r.lock();
                json!({
                    "channel": r.login,
                    "state": i.state,
                    "events": i.log.seq(),
                    "idle_seconds": (r.idle().as_secs_f64() * 10.0).round() / 10.0,
                })
            })
            .collect()
    }
}

// -- emotes -------------------------------------------------------------------------------------

const FETCH_TIMEOUT: Duration = Duration::from_secs(6);
const ALL_TIMEOUT: Duration = Duration::from_secs(12);
const CACHE_MAX: usize = 200;
const CACHE_TRIM: usize = 50;
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

struct Cached {
    at: Instant,
    /// How long the answer is good for: the list's age, or a minute after a failure.
    keep: Duration,
    got: Table,
    err: String,
}

/// The three providers' lists, fetched straight out and kept: a global list an hour, a channel's
/// ten minutes, a failure one minute.
pub struct Emotes {
    client: reqwest::Client,
    rewrite: Option<String>,
    cache: Mutex<BTreeMap<String, Cached>>,
}

impl Emotes {
    /// `rewrite`, set only by tests, replaces `https://` in every provider address with a base of
    /// its own, so `https://host/path` is asked as `{rewrite}/host/path`.
    pub fn new(rewrite: Option<String>) -> Result<Self, reqwest::Error> {
        crate::twitch::install_tls();
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent(UA)
                .timeout(FETCH_TIMEOUT)
                .no_proxy()
                .build()?,
            rewrite,
            cache: Mutex::new(BTreeMap::new()),
        })
    }

    fn url(&self, url: &str) -> String {
        match &self.rewrite {
            Some(base) => format!("{base}/{}", url.trim_start_matches("https://")),
            None => url.to_string(),
        }
    }

    async fn fetch(&self, url: &str) -> Result<Value, FetchError> {
        let r = self
            .client
            .get(self.url(url))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| FetchError::Other(e.to_string()))?;
        let code = r.status().as_u16();
        if !r.status().is_success() {
            return Err(FetchError::Http(code));
        }
        let bytes = r
            .bytes()
            .await
            .map_err(|e| FetchError::Other(e.to_string()))?;
        if bytes.len() > 4_000_000 {
            return Err(FetchError::Other("answer too long".to_string()));
        }
        serde_json::from_slice(&bytes).map_err(|e| FetchError::Other(e.to_string()))
    }

    /// One list, from the cache when it is fresh. A 404 is a channel with no list there, not a
    /// fault; any other failure keeps what was had and is asked again in a minute.
    async fn cached(
        &self,
        source: &breakless_formats::emotes::Source,
        scope: Scope,
        url: &str,
    ) -> (Table, String) {
        let age = Duration::from_secs(match scope {
            Scope::Global => GLOBAL_AGE,
            Scope::Channel => CHANNEL_AGE,
        });
        let hit = {
            let cache = self.lock();
            cache
                .get(url)
                .map(|c| (c.at.elapsed() < c.keep, c.got.clone(), c.err.clone()))
        };
        if let Some((true, got, err)) = &hit {
            return (got.clone(), err.clone());
        }
        let (got, err) = match self.fetch(url).await {
            Ok(v) => (source.read(&v), String::new()),
            Err(FetchError::Http(404)) => (Table::new(), String::new()),
            Err(FetchError::Http(code)) => (Table::new(), format!("HTTP {code}")),
            Err(FetchError::Other(e)) => (
                hit.as_ref().map(|h| h.1.clone()).unwrap_or_default(),
                truncate(&e, 80),
            ),
        };
        let mut cache = self.lock();
        cache.insert(
            url.to_string(),
            Cached {
                at: Instant::now(),
                keep: if err.is_empty() {
                    age
                } else {
                    Duration::from_secs(RETRY_AGE)
                },
                got: got.clone(),
                err: err.clone(),
            },
        );
        if cache.len() > CACHE_MAX {
            let mut by_age: Vec<(Instant, String)> =
                cache.iter().map(|(k, c)| (c.at, k.clone())).collect();
            by_age.sort();
            for (_, k) in by_age.into_iter().take(CACHE_TRIM) {
                cache.remove(&k);
            }
        }
        (got, err)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Cached>> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `/api/emotes/<channel>`: the merged table, counts per provider and scope, and the errors
    /// by provider name. An empty `room_id` (chat not joined yet) gives the globals alone.
    pub async fn emotes(&self, room_id: &str) -> Value {
        let scopes: &[Scope] = if valid_room_id(room_id) {
            &[Scope::Global, Scope::Channel]
        } else {
            &[Scope::Global]
        };
        let jobs: Vec<(Scope, &breakless_formats::emotes::Source)> = scopes
            .iter()
            .flat_map(|s| SOURCES.iter().map(move |src| (*s, src)))
            .collect();
        // six lists at once: one slow provider does not hold the rest
        let asked = jobs.iter().map(|(scope, src)| {
            let url = src.url(*scope, room_id);
            async move {
                tokio::time::timeout(ALL_TIMEOUT, self.cached(src, *scope, &url))
                    .await
                    .unwrap_or_else(|_| (Table::new(), "no answer in time".to_string()))
            }
        });
        let answers = futures_join_all(asked).await;
        let mut errors = serde_json::Map::new();
        let mut merged = Vec::new();
        for ((scope, src), (got, err)) in jobs.iter().zip(answers) {
            if !err.is_empty() {
                errors.insert(
                    format!("{} {scope}", provider_name(src.provider)),
                    Value::String(err),
                );
            }
            merged.push((*scope, src.provider, got));
        }
        let (table, counts) = merge(&merged);
        json!({"room_id": room_id, "emotes": table, "counts": counts, "errors": errors})
    }
}

enum FetchError {
    Http(u16),
    Other(String),
}

/// Runs the futures together and returns their results in order. Twenty lines instead of a crate.
async fn futures_join_all<F: Future>(futures: impl IntoIterator<Item = F>) -> Vec<F::Output> {
    let mut futures: Vec<std::pin::Pin<Box<F>>> = futures.into_iter().map(Box::pin).collect();
    let mut outputs: Vec<Option<F::Output>> = (0..futures.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut all = true;
        for (f, out) in futures.iter_mut().zip(outputs.iter_mut()) {
            if out.is_none() {
                match f.as_mut().poll(cx) {
                    std::task::Poll::Ready(v) => *out = Some(v),
                    std::task::Poll::Pending => all = false,
                }
            }
        }
        if all {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await;
    outputs
        .into_iter()
        .map(|o| o.expect("every future finished"))
        .collect()
}
