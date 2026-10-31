//! A fake Twitch on loopback for the service's tests: a GraphQL endpoint, a playlist server and
//! a media server whose playlists advance only when a test says so.
//!
//! Each channel has a script: one flag per 2-second segment, `true` for an advert. The media
//! playlist shows a window of five segments ending at the channel's position; `advance` moves it.
//!
//! Beside those: a chat server on a plain socket (it answers a `JOIN` with `ROOMSTATE`, records
//! what clients send, and says whatever a test tells it to) and the three emote providers'
//! lists, keyed by a piece of the address, as the prototype's tests key them.

#![allow(
    dead_code,
    missing_docs,
    clippy::missing_panics_doc,
    clippy::needless_pass_by_value
)] // a test fixture, not an API

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct Script {
    pub ads: Vec<bool>,
    pub pos: usize,
    pub offline: bool,
    pub name: String,
    pub viewers: u64,
}

#[derive(Default)]
pub struct Inner {
    pub channels: BTreeMap<String, Script>,
    pub fail_media_next: usize,
    pub country: String,
    pub requests: Vec<String>,
    /// Lines chat clients sent, in order.
    pub irc_received: Vec<String>,
    /// `(piece of the address, status)`: an emote list that answers that status instead.
    pub emote_status: Vec<(String, u16)>,
    /// Emote list addresses asked, in order.
    pub emote_requests: Vec<String>,
}

#[derive(Clone)]
pub struct FakeTwitch {
    pub base: String,
    pub irc: SocketAddr,
    pub inner: Arc<Mutex<Inner>>,
    say: broadcast::Sender<String>,
}

const WINDOW: usize = 5;

impl FakeTwitch {
    pub async fn start() -> Self {
        let inner = Arc::new(Mutex::new(Inner {
            country: "CA".to_string(),
            ..Inner::default()
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let irc_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let irc = irc_listener.local_addr().unwrap();
        let (say, _) = broadcast::channel(64);
        let fake = Self {
            base: format!("http://{addr}"),
            irc,
            inner,
            say,
        };
        let app = Router::new()
            .route("/gql", post(gql))
            .route("/api/channel/hls/{file}", get(master))
            .route("/media/{login}/{file}", get(media))
            .route("/emotes/{*rest}", get(emotes))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let chat = fake.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = irc_listener.accept().await else {
                    break;
                };
                tokio::spawn(chat.clone().irc_client(socket));
            }
        });
        fake
    }

    /// Where the chat server is, for `--irc-url`.
    pub fn irc_url(&self) -> String {
        format!("tcp://{}", self.irc)
    }

    /// Where the emote lists are, for `--emotes-url`.
    pub fn emotes_url(&self) -> String {
        format!("{}/emotes", self.base)
    }

    /// Says a line to every connected chat client.
    pub fn irc_say(&self, line: &str) {
        let _ = self.say.send(line.to_string());
    }

    pub fn irc_received(&self) -> Vec<String> {
        self.inner.lock().unwrap().irc_received.clone()
    }

    /// Emote list addresses containing `piece` answer `status` from now on.
    pub fn emote_status(&self, piece: &str, status: u16) {
        self.inner
            .lock()
            .unwrap()
            .emote_status
            .push((piece.to_string(), status));
    }

    pub fn emote_requests(&self) -> Vec<String> {
        self.inner.lock().unwrap().emote_requests.clone()
    }

    async fn irc_client(self, socket: tokio::net::TcpStream) {
        let (reader, mut writer) = socket.into_split();
        let mut lines = BufReader::new(reader).lines();
        let mut say = self.say.subscribe();
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { break };
                    self.inner.lock().unwrap().irc_received.push(line.clone());
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

    pub fn gql_url(&self) -> String {
        format!("{}/gql", self.base)
    }

    pub fn usher_url(&self) -> String {
        format!("{}/api/channel/hls/{{login}}.m3u8", self.base)
    }

    /// A live channel whose first `pos` segments are already in view. `ads` has one flag per
    /// segment; segments past the script's end are stream.
    pub fn channel(&self, login: &str, ads: &[bool], pos: usize) {
        let mut i = self.inner.lock().unwrap();
        i.channels.insert(
            login.to_string(),
            Script {
                ads: ads.to_vec(),
                pos,
                offline: false,
                name: login.to_uppercase(),
                viewers: 100,
            },
        );
    }

    pub fn offline(&self, login: &str) {
        let mut i = self.inner.lock().unwrap();
        i.channels
            .entry(login.to_string())
            .or_insert(Script {
                ads: vec![],
                pos: 0,
                offline: true,
                name: login.to_uppercase(),
                viewers: 0,
            })
            .offline = true;
    }

    /// Moves a channel's window on by `n` segments.
    pub fn advance(&self, login: &str, n: usize) {
        let mut i = self.inner.lock().unwrap();
        if let Some(c) = i.channels.get_mut(login) {
            c.pos += n;
        }
    }

    /// The next `n` media requests answer 503.
    pub fn fail_media_next(&self, n: usize) {
        self.inner.lock().unwrap().fail_media_next = n;
    }

    pub fn requests(&self) -> Vec<String> {
        self.inner.lock().unwrap().requests.clone()
    }
}

async fn gql(State(fake): State<FakeTwitch>, body: Bytes) -> Response {
    let req: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let query = req["query"].as_str().unwrap_or("");
    let mut inner = fake.inner.lock().unwrap();
    inner.requests.push("gql".to_string());
    if query.contains("PlaybackAccessToken") {
        let login = req["variables"]["login"]
            .as_str()
            .unwrap_or("")
            .to_lowercase();
        let token = match inner.channels.get(&login) {
            Some(_) => json!({"value": format!("tok-{login}"), "signature": "sig"}),
            None => Value::Null,
        };
        return j(json!({"data": {"streamPlaybackAccessToken": token}}));
    }
    if query.contains("users(logins") {
        let users: Vec<Value> = req["variables"]["logins"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| l.as_str())
            .map(|l| match inner.channels.get(l) {
                Some(c) if !c.offline => json!({"login": l, "displayName": c.name,
                    "stream": {"title": format!("{l} live"), "viewersCount": c.viewers, "game": {"displayName": "Games"}}}),
                _ => json!({"login": l, "displayName": l.to_uppercase(), "stream": null}),
            })
            .collect();
        return j(json!({"data": {"users": users}}));
    }
    if query.contains("streams(first") {
        let edges: Vec<Value> = inner
            .channels
            .iter()
            .filter(|(_, c)| !c.offline)
            .map(|(l, c)| json!({"node": {"title": format!("{l} live"), "viewersCount": c.viewers,
                "broadcaster": {"login": l, "displayName": c.name}, "game": {"displayName": "Games"}}}))
            .collect();
        return j(json!({"data": {"streams": {"edges": edges}}}));
    }
    j(json!({"errors": [{"message": "unknown query"}]}))
}

async fn master(State(fake): State<FakeTwitch>, Path(file): Path<String>) -> Response {
    let login = file.trim_end_matches(".m3u8").to_string();
    let mut inner = fake.inner.lock().unwrap();
    inner.requests.push(format!("master {login}"));
    match inner.channels.get(&login) {
        Some(c) if !c.offline => {
            let text = format!(
                "#EXTM3U\n#EXT-X-TWITCH-INFO:NODE=\"fake\",USER-COUNTRY=\"{cc}\",CLUSTER=\"fake_cluster\"\n\
                 #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60 (source)\",AUTOSELECT=YES,DEFAULT=YES\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,CODECS=\"avc1.64002A,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\n\
                 {base}/media/{login}/chunked.m3u8\n\
                 #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"720p60\",NAME=\"720p60\",AUTOSELECT=YES,DEFAULT=YES\n\
                 #EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"720p60\",FRAME-RATE=60.000\n\
                 {base}/media/{login}/720p60.m3u8\n",
                cc = inner.country,
                base = fake.base
            );
            ([("Content-Type", "application/vnd.apple.mpegurl")], text).into_response()
        }
        _ => (StatusCode::NOT_FOUND, "Can not find channel").into_response(),
    }
}

async fn media(
    State(fake): State<FakeTwitch>,
    Path((login, file)): Path<(String, String)>,
) -> Response {
    let mut inner = fake.inner.lock().unwrap();
    inner.requests.push(format!("media {login}/{file}"));
    if inner.fail_media_next > 0 {
        inner.fail_media_next -= 1;
        return (StatusCode::SERVICE_UNAVAILABLE, "fake outage").into_response();
    }
    let Some(c) = inner.channels.get(&login).filter(|c| !c.offline) else {
        return (StatusCode::NOT_FOUND, "gone").into_response();
    };
    let start = c.pos.saturating_sub(WINDOW);
    let is_ad = |i: usize| c.ads.get(i).copied().unwrap_or(false);
    let mut out = vec![
        "#EXTM3U".to_string(),
        "#EXT-X-VERSION:3".to_string(),
        "#EXT-X-TARGETDURATION:6".to_string(),
        format!("#EXT-X-MEDIA-SEQUENCE:{start}"),
    ];
    // one DATERANGE per advert run in view, as Twitch writes them
    let mut i = start;
    while i < c.pos {
        if is_ad(i) {
            let first = i;
            while i < c.pos && is_ad(i) {
                i += 1;
            }
            out.push(format!(
                "#EXT-X-DATERANGE:ID=\"stitched-ad-{first}\",CLASS=\"twitch-stitched-ad\",START-DATE=\"{}\",DURATION={}.000,X-TV-TWITCH-AD-ROLL-TYPE=\"{}\",X-TV-TWITCH-AD-POD-FILLED-DURATION=\"{}\"",
                when(first),
                2 * (i - first),
                if first == 0 { "PREROLL" } else { "MIDROLL" },
                2 * (i - first)
            ));
        } else {
            i += 1;
        }
    }
    let mut prev: Option<bool> = None;
    for i in start..c.pos {
        let ad = is_ad(i);
        if prev.is_some_and(|p| p != ad) {
            out.push("#EXT-X-DISCONTINUITY".to_string());
        }
        prev = Some(ad);
        out.push(format!("#EXT-X-PROGRAM-DATE-TIME:{}", when(i)));
        out.push(format!(
            "#EXTINF:2.000,{}",
            if ad { "Amazon|1" } else { "live" }
        ));
        out.push(format!(
            "http://cdn.example/{login}/{}-{i}.ts",
            if ad { "ad" } else { "live" }
        ));
    }
    let mut text = out.join("\n");
    text.push('\n');
    ([("Content-Type", "application/vnd.apple.mpegurl")], text).into_response()
}

/// The prototype's test tables (`reference/python/test_chat.py`, `EmoteTests`), chosen by a
/// piece of the address.
async fn emotes(State(fake): State<FakeTwitch>, Path(rest): Path<String>) -> Response {
    let mut inner = fake.inner.lock().unwrap();
    inner.emote_requests.push(rest.clone());
    if let Some((_, status)) = inner.emote_status.iter().find(|(p, _)| rest.contains(p)) {
        return (
            StatusCode::from_u16(*status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            "no",
        )
            .into_response();
    }
    let table: &[(&str, Value)] = &[
        (
            "betterttv.net/3/cached/emotes/global",
            json!([{"id": "54fa8f1401e468494b85b537", "code": ":tf:", "imageType": "png", "modifier": false}, {"id": "bad id!", "code": "Nope"}]),
        ),
        (
            "betterttv.net/3/cached/users",
            json!({"channelEmotes": [{"id": "aaa", "code": "catJAM"}], "sharedEmotes": [{"id": "bbb", "code": "monkaS"}]}),
        ),
        (
            "frankerfacez.com/v1/set/global",
            json!({"default_sets": [3], "sets": {"3": {"emoticons": [{"id": 9, "name": "ZrehplaR"}]}, "4": {"emoticons": [{"id": 10, "name": "NotDefault"}]}}}),
        ),
        (
            "frankerfacez.com/v1/room",
            json!({"room": {}, "sets": {"166907": {"emoticons": [{"id": 77, "name": "catJAM"}, {"id": 78, "name": "OnlyFFZ"}]}}}),
        ),
        (
            "7tv.io/v3/emote-sets/global",
            json!({"emotes": [{"id": "01A", "name": "reckH", "flags": 0}, {"id": "01B", "name": "RainTime", "flags": 1}]}),
        ),
        (
            "7tv.io/v3/users",
            json!({"emote_set": {"emotes": [{"id": "01C", "name": "catJAM", "flags": 0}]}}),
        ),
    ];
    match table.iter().find(|(piece, _)| rest.contains(piece)) {
        Some((_, v)) => j(v.clone()),
        None => (StatusCode::NOT_FOUND, "no such list").into_response(),
    }
}

fn when(i: usize) -> String {
    let s = 2 * i;
    format!("2026-10-04T12:{:02}:{:02}.000Z", s / 60, s % 60)
}

fn j(v: Value) -> Response {
    ([("Content-Type", "application/json")], v.to_string()).into_response()
}
