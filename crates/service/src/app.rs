//! Shared state: the channels being watched, the totals, the channel list, the files.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use breakless_formats::Media;
use serde_json::{Value, json};

use crate::channel::{Channel, EXIT, HOME, IDLE};
use crate::config::Config;
use crate::store::Store;
use crate::twitch::Twitch;

const LISTING_AGE: Duration = Duration::from_secs(30);

/// Counters since start, by where the playlist was asked from.
#[derive(Default)]
struct Counts {
    breaks: BTreeMap<&'static str, u64>,
    client_side: u64,
    cut: BTreeMap<&'static str, u64>,
    passed: u64,
    sessions: BTreeMap<&'static str, u64>,
    odd_titles: BTreeSet<String>,
    odd_classes: BTreeSet<String>,
}

/// The totals in `/api/status`.
#[derive(Default)]
pub struct Totals(Mutex<Counts>);

impl Totals {
    fn with<R>(&self, f: impl FnOnce(&mut Counts) -> R) -> R {
        f(&mut self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    /// One more session opened from `mode`.
    pub fn session(&self, mode: &'static str) {
        self.with(|c| *c.sessions.entry(mode).or_default() += 1);
    }

    /// One more advert break met on a `mode` session.
    pub fn advert_break(&self, mode: &'static str) {
        self.with(|c| *c.breaks.entry(mode).or_default() += 1);
    }

    /// One more time Twitch asked the player to draw its own advert.
    pub fn client_side(&self) {
        self.with(|c| c.client_side += 1);
    }

    /// One playlist fed to a timeline: what was passed on, what was cut, what was odd.
    pub fn fed(&self, mode: &'static str, added: u64, cut: u64, parsed: &Media) {
        self.with(|c| {
            c.passed += added;
            *c.cut.entry(mode).or_default() += cut;
            c.odd_titles.extend(parsed.odd_titles.iter().cloned());
            c.odd_classes.extend(parsed.odd_classes.iter().cloned());
        });
    }

    fn json(&self) -> Value {
        self.with(|c| {
            let by_mode = |m: &BTreeMap<&str, u64>| json!({EXIT: m.get(EXIT).copied().unwrap_or(0), HOME: m.get(HOME).copied().unwrap_or(0)});
            json!({
                "advert_breaks": by_mode(&c.breaks),
                "advert_segments_cut": by_mode(&c.cut),
                "client_side_advert_requests_ignored": c.client_side,
                "stream_segments_passed": c.passed,
                "sessions": by_mode(&c.sessions),
                "segment_titles_not_seen_before": c.odd_titles,
                "daterange_classes_not_seen_before": c.odd_classes,
            })
        })
    }

    /// Advert segments cut on every line, for `/api/watch`.
    fn cut_total(&self) -> u64 {
        self.with(|c| c.cut.values().sum())
    }
}

/// Everything the routes share.
pub struct App {
    /// What the service was started with.
    pub config: Config,
    /// The way to Twitch, straight and through the exits.
    pub twitch: Twitch,
    /// The two files on disk.
    pub store: Store,
    /// The counters in `/api/status`.
    pub totals: Totals,
    started: Instant,
    channels: Mutex<BTreeMap<String, Arc<Channel>>>,
    listing: tokio::sync::Mutex<Option<(Instant, Value)>>,
}

impl App {
    /// Builds the HTTP clients; fails only if a proxy address does not parse.
    pub fn new(config: Config) -> Result<Self, reqwest::Error> {
        let twitch = Twitch::new(&config.proxies(), &config.gql_url, &config.usher_url)?;
        let store = Store::new(&config.data);
        Ok(Self {
            config,
            twitch,
            store,
            totals: Totals::default(),
            started: Instant::now(),
            channels: Mutex::new(BTreeMap::new()),
            listing: tokio::sync::Mutex::new(None),
        })
    }

    /// The channel for a name, created on first sight.
    pub fn channel(&self, login: &str) -> Arc<Channel> {
        let login = login.to_lowercase();
        let mut map = self
            .channels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.entry(login.clone())
            .or_insert_with(|| Arc::new(Channel::new(&login)))
            .clone()
    }

    fn channels(&self) -> Vec<Arc<Channel>> {
        self.channels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Drops channels nobody asked for in [`IDLE`].
    pub fn reap(&self) {
        let mut map = self
            .channels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|login, c| {
            let keep = c.idle() <= IDLE;
            if !keep {
                tracing::info!("{login}: nobody watching, session dropped");
            }
            keep
        });
    }

    /// The first exit's part of `/api/status` (what the page's lamp reads) and all of them.
    fn exits_json(&self) -> (Value, Vec<Value>) {
        let all: Vec<Value> = self
            .twitch
            .exits
            .iter()
            .map(crate::twitch::Exit::status)
            .collect();
        let first = self
            .twitch
            .first_usable_exit()
            .or(if all.is_empty() { None } else { Some(0) });
        let summary = match first {
            Some(i) => all[i].clone(),
            None => {
                json!({"configured": false, "usable": false, "country": "", "cluster": "", "last_ok_seconds_ago": null, "last_error": ""})
            }
        };
        (summary, all)
    }

    /// `/api/status`.
    pub async fn status(&self) -> Value {
        let mut watching = Vec::new();
        for c in self.channels() {
            if c.has_session().await {
                watching.push(c.status().await);
            }
        }
        let (exit, exits) = self.exits_json();
        json!({
            "up_seconds": self.started.elapsed().as_secs(),
            "exit": exit,
            "exits": exits,
            "watching": watching,
            "chat": [],
            "totals": self.totals.json(),
        })
    }

    /// `/api/watch/<channel>`: what one viewer's player shows.
    pub async fn watch(&self, login: &str) -> Value {
        let login = login.to_lowercase();
        let c = self
            .channels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&login)
            .cloned();
        let mine = match c {
            Some(c) if c.has_session().await => Some(c.status().await),
            _ => None,
        };
        let (exit, _) = self.exits_json();
        json!({
            "channel": login,
            "exit": {"configured": exit["configured"], "usable": exit["usable"], "country": exit["country"]},
            "from": mine.as_ref().map_or(Value::Null, |m| m["from"].clone()),
            "country": mine.as_ref().map_or(Value::Null, |m| m["country"].clone()),
            "advert_break": mine.as_ref().map_or(Value::Null, |m| m["advert_break"].clone()),
            "segments_cut": mine.as_ref().map_or(0, |m| m["segments_cut"].as_u64().unwrap_or(0)),
            "segments_cut_since_start": self.totals.cut_total(),
        })
    }

    /// Forgets the cached channel list (after a follow changed).
    pub async fn forget_listing(&self) {
        *self.listing.lock().await = None;
    }

    /// `/api/channels`: followed channels (live first) and the most watched. Asked from Twitch
    /// twice a minute at most.
    pub async fn listing(&self) -> Value {
        let mut cache = self.listing.lock().await;
        if let Some((at, v)) = cache.as_ref()
            && at.elapsed() < LISTING_AGE
        {
            return v.clone();
        }
        let mine = self.store.follows();
        let fetched = async {
            let on = self.twitch.live(&mine).await?;
            let top = self.twitch.top(12).await?;
            Ok::<_, crate::twitch::Error>((on, top))
        }
        .await;
        let out = match fetched {
            Ok((on, top)) => {
                let mut follows: Vec<Value> = mine
                    .iter()
                    .map(|l| match on.get(l) {
                        Some(live) => json!({"login": l, "live": true, "name": live.name, "title": live.title, "viewers": live.viewers, "game": live.game}),
                        None => json!({"login": l, "live": false, "name": l}),
                    })
                    .collect();
                follows.sort_by(|a, b| {
                    let key = |c: &Value| {
                        (
                            !c["live"].as_bool().unwrap_or(false),
                            std::cmp::Reverse(c["viewers"].as_u64().unwrap_or(0)),
                            c["login"].as_str().unwrap_or("").to_string(),
                        )
                    };
                    key(a).cmp(&key(b))
                });
                json!({"follows": follows, "top": top, "error": null, "host": self.config.host})
            }
            Err(e) => json!({
                "follows": mine.iter().map(|l| json!({"login": l, "live": null, "name": l})).collect::<Vec<_>>(),
                "top": [], "error": crate::twitch::truncate(&e.message, 200), "host": self.config.host,
            }),
        };
        *cache = Some((Instant::now(), out.clone()));
        out
    }
}
