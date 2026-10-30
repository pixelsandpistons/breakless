//! One channel being watched: its session, its rebuilt playlists, the advert break under way.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use breakless_formats::{Media, Timeline, Variant, master_info, parse_media, variants};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::app::App;
use crate::twitch::{self, Error, truncate};

/// Where a session's playlists are asked from.
pub const EXIT: &str = "exit";
/// The home line, when no exit answers.
pub const HOME: &str = "home";
/// Seconds a quality's playlist is reused before asking Twitch again.
const REFRESH: Duration = Duration::from_millis(900);
/// Seconds without a request before a channel's session is dropped.
pub const IDLE: Duration = Duration::from_secs(90);

/// The channel is not live, or does not exist.
#[derive(Debug)]
pub struct Offline;

/// Why a playlist could not be served.
#[derive(Debug)]
pub enum Fault {
    /// The channel is not live, or does not exist: 404.
    Offline,
    /// Twitch, or the way to it, failed: 502.
    Twitch(Error),
}

impl From<Error> for Fault {
    fn from(e: Error) -> Self {
        Self::Twitch(e)
    }
}

/// One token and one master playlist: where it was asked from and what Twitch offered.
pub struct Session {
    /// `login-N`, N counting the channel's sessions since start.
    pub id: String,
    /// [`EXIT`] or [`HOME`].
    pub mode: &'static str,
    /// Index into the exits, or `None` for the home line.
    pub exit: Option<usize>,
    /// The qualities by slug.
    pub variants: BTreeMap<String, Variant>,
    /// The slugs, best first.
    pub order: Vec<String>,
    /// Where Twitch placed the request.
    pub country: String,
    /// The CDN cluster serving the video.
    pub cluster: String,
}

struct Break {
    since: Instant,
    roll: String,
    seconds: String,
    cut: u64,
    mode: &'static str,
    country: String,
}

#[derive(Default)]
struct State {
    session: Option<Arc<Session>>,
    sessions_opened: u32,
    retry_exit: Option<Instant>,
    in_break: Option<Break>,
    client_side: bool,
    asked: BTreeSet<String>,
    cut: u64,
    passed: u64,
}

struct Quality {
    timeline: Timeline,
    fetched: Option<Instant>,
}

/// A channel somebody is watching.
pub struct Channel {
    /// The channel's name, lower case.
    pub login: String,
    used: std::sync::Mutex<Instant>,
    state: Mutex<State>,
    qualities: Mutex<BTreeMap<String, Arc<Mutex<Quality>>>>,
}

impl Channel {
    /// A channel nobody has asked for yet.
    pub fn new(login: &str) -> Self {
        Self {
            login: login.to_string(),
            used: std::sync::Mutex::new(Instant::now()),
            state: Mutex::new(State::default()),
            qualities: Mutex::new(BTreeMap::new()),
        }
    }

    fn touch(&self) {
        *self
            .used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
    }

    /// Seconds since the last request for this channel.
    pub fn idle(&self) -> Duration {
        self.used
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .elapsed()
    }

    /// Asks Twitch for a master playlist: through the first usable exit (twice, since about one
    /// request in a few hundred dies in transit and falling back for that costs a viewer a
    /// pre-roll's worth of frozen picture), then the other usable exits, then the home line.
    async fn open(&self, app: &App, state: &mut State) -> Result<Arc<Session>, Fault> {
        let t = &app.twitch;
        let mut attempts: Vec<(&'static str, Option<usize>)> = Vec::new();
        for (i, e) in t.exits.iter().enumerate() {
            if e.usable() {
                attempts.push((EXIT, Some(i)));
                if attempts.len() == 1 {
                    attempts.push((EXIT, Some(i)));
                }
            }
        }
        attempts.push((HOME, None));
        let mut last: Option<Error> = None;
        for (n, (mode, exit)) in attempts.iter().enumerate() {
            let text = match t.master(t.via(*exit), &self.login).await {
                Ok(text) => text,
                Err(e) => {
                    // usher answers 404 for a channel that is not live; no token at all means no such channel
                    if e.code == 404 || e.message.starts_with("no token") {
                        return Err(Fault::Offline);
                    }
                    if let Some(i) = exit {
                        if attempts.get(n + 1).is_some_and(|next| next.1 == *exit) {
                            tracing::info!(
                                "{}: the exit did not answer ({}), asking it once more",
                                self.login,
                                truncate(&e.message, 80)
                            );
                        } else {
                            t.exits[*i].failed(&e.message);
                        }
                    }
                    last = Some(e);
                    continue;
                }
            };
            let v = variants(&text);
            if v.is_empty() {
                last = Some(Error {
                    code: 0,
                    message: "no qualities in the master playlist".to_string(),
                });
                continue;
            }
            let info = master_info(&text);
            state.sessions_opened += 1;
            let session = Arc::new(Session {
                id: format!("{}-{}", self.login, state.sessions_opened),
                mode,
                exit: *exit,
                order: v.iter().map(|x| x.slug.clone()).collect(),
                variants: v.into_iter().map(|x| (x.slug.clone(), x)).collect(),
                country: info.country.clone(),
                cluster: info.cluster.clone(),
            });
            app.totals.session(mode);
            if let Some(i) = exit {
                t.exits[*i].ok(&info.country, &info.cluster);
            }
            tracing::info!(
                "{}: session from {mode}, Twitch sees {}, {} qualities",
                self.login,
                info.country,
                session.order.len()
            );
            state.session = Some(session.clone());
            return Ok(session);
        }
        Err(Fault::Twitch(last.unwrap_or(Error {
            code: 0,
            message: "no session".to_string(),
        })))
    }

    /// The master playlist handed to players: our addresses, Twitch's names.
    pub async fn master(&self, app: &App) -> Result<String, Fault> {
        self.touch();
        let mut state = self.state.lock().await;
        let s = match &state.session {
            Some(s) => s.clone(),
            None => self.open(app, &mut state).await?,
        };
        drop(state);
        let mut out = vec!["#EXTM3U".to_string()];
        for slug in &s.order {
            let v = &s.variants[slug];
            out.push(format!("#EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"{slug}\",NAME=\"{}\",AUTOSELECT=YES,DEFAULT=YES", v.name));
            let resolution = if v.resolution.is_empty() {
                String::new()
            } else {
                format!(",RESOLUTION={}", v.resolution)
            };
            let codecs = if v.codecs.is_empty() {
                String::new()
            } else {
                format!(",CODECS=\"{}\"", v.codecs)
            };
            let fps = if v.fps.is_empty() {
                String::new()
            } else {
                format!(",FRAME-RATE={}", v.fps)
            };
            out.push(format!(
                "#EXT-X-STREAM-INF:BANDWIDTH={}{resolution}{codecs},VIDEO=\"{slug}\"{fps}",
                v.bandwidth
            ));
            out.push(format!("{}/{slug}.m3u8", self.login));
        }
        let mut text = out.join("\n");
        text.push('\n');
        Ok(text)
    }

    async fn quality(&self, slug: &str) -> Arc<Mutex<Quality>> {
        let mut q = self.qualities.lock().await;
        q.entry(slug.to_string())
            .or_insert_with(|| {
                Arc::new(Mutex::new(Quality {
                    timeline: Timeline::default(),
                    fetched: None,
                }))
            })
            .clone()
    }

    /// One quality's rebuilt playlist, refreshed from Twitch at most every 0.9 s.
    pub async fn media(&self, app: &App, slug: &str) -> Result<String, Fault> {
        self.touch();
        let quality = self.quality(slug).await;
        let mut quality = quality.lock().await;
        let now = Instant::now();
        if quality
            .fetched
            .is_some_and(|at| now.duration_since(at) < REFRESH)
        {
            return Ok(quality.timeline.render());
        }
        quality.fetched = Some(now);
        let twitch = &app.twitch;
        let mut result: Option<(Arc<Session>, Media)> = None;
        for attempt in 1..=3 {
            let session = {
                let mut state = self.state.lock().await;
                // A session that had to be opened from home goes back to the exit as soon as it answers.
                if state.session.as_ref().is_some_and(|cur| cur.mode == HOME)
                    && twitch.first_usable_exit().is_some()
                    && state.retry_exit.is_none_or(|at| now >= at)
                {
                    state.retry_exit = Some(now + twitch::EXIT_REST);
                    state.session = None;
                }
                match &state.session {
                    Some(cur) => cur.clone(),
                    None => self.open(app, &mut state).await?,
                }
            };
            let variant = session.variants.get(slug).or_else(|| {
                session
                    .order
                    .first()
                    .and_then(|first| session.variants.get(first))
            });
            let Some(variant) = variant else {
                return Err(Fault::Twitch(Error {
                    code: 0,
                    message: "no qualities in the master playlist".to_string(),
                }));
            };
            match twitch
                .http(twitch.via(session.exit), &variant.url, None)
                .await
            {
                Ok(text) => {
                    result = Some((session, parse_media(&text)));
                    break;
                }
                Err(e) => {
                    tracing::info!("{}/{slug}: {}", self.login, truncate(&e.message, 100));
                    // About one request in a few hundred through the exit dies in transit. Ask once
                    // more before giving the session up.
                    if e.is_passing() && attempt == 1 {
                        continue;
                    }
                    // Otherwise the session is over: Twitch said so (403/404), or the exit is away.
                    if e.is_passing()
                        && let Some(i) = session.exit
                    {
                        twitch.exits[i].failed(&e.message);
                    }
                    let mut state = self.state.lock().await;
                    if state
                        .session
                        .as_ref()
                        .is_some_and(|cur| cur.id == session.id)
                    {
                        state.session = None;
                    }
                    if attempt == 3 && quality.timeline.is_empty() {
                        return Err(Fault::Twitch(e));
                    }
                }
            }
        }
        let Some((session, parsed)) = result else {
            return Ok(quality.timeline.render());
        };
        let (added, cut) = quality.timeline.feed(&session.id, &parsed);
        let mut state = self.state.lock().await;
        state.asked.insert(slug.to_string());
        state.passed += added;
        state.cut += cut;
        app.totals.fed(session.mode, added, cut, &parsed);
        if let Some(record) = self.note_adverts(app, &mut state, &session, &parsed, cut) {
            app.store.record(record);
        }
        Ok(quality.timeline.render())
    }

    /// Tracks the break under way; returns the record to write when one ends.
    fn note_adverts(
        &self,
        app: &App,
        state: &mut State,
        s: &Session,
        parsed: &Media,
        cut: u64,
    ) -> Option<Value> {
        let now = Instant::now();
        if parsed.client_side != state.client_side {
            state.client_side = parsed.client_side;
            if state.client_side {
                app.totals.client_side();
                tracing::info!(
                    "{}: Twitch asks the player to show an advert of its own; ignored, stream untouched",
                    self.login
                );
            }
        }
        // A break lasts while the newest segment is an advert. (The advert's DATERANGE tag stays
        // in the playlist for half a minute after the stream is back, so it cannot be the test.)
        let newest_is_ad = parsed.segments.last().is_some_and(|seg| seg.ad);
        if let Some(ads) = parsed.ads.as_ref().filter(|_| newest_is_ad) {
            if state.in_break.is_none() {
                state.in_break = Some(Break {
                    since: now,
                    roll: ads.roll.clone(),
                    seconds: ads.seconds.clone(),
                    cut: 0,
                    mode: s.mode,
                    country: s.country.clone(),
                });
                app.totals.advert_break(s.mode);
                tracing::info!(
                    "{}: advert break ({}, {}s) on a {} session, cutting",
                    self.login,
                    ads.roll,
                    if ads.seconds.is_empty() {
                        "?"
                    } else {
                        &ads.seconds
                    },
                    s.mode
                );
            }
            if let Some(b) = state.in_break.as_mut() {
                b.cut += cut;
            }
            None
        } else if let Some(b) = state.in_break.take() {
            let held = b.since.elapsed().as_secs_f64();
            tracing::info!(
                "{}: advert break over after {held:.0}s, {} segments cut",
                self.login,
                b.cut
            );
            Some(json!({
                "channel": self.login, "from": b.mode, "country": b.country, "roll": b.roll,
                "advertised_seconds": b.seconds, "held_seconds": (held * 10.0).round() / 10.0,
                "segments_cut": b.cut,
            }))
        } else {
            None
        }
    }

    /// `true` when a session is open (the channel appears in `/api/status`).
    pub async fn has_session(&self) -> bool {
        self.state.lock().await.session.is_some()
    }

    /// This channel's part of `/api/status`.
    pub async fn status(&self) -> Value {
        let state = self.state.lock().await;
        let s = state.session.as_ref();
        json!({
            "channel": self.login,
            "from": s.map(|s| s.mode),
            "country": s.map(|s| s.country.clone()),
            "qualities": s.map_or_else(Vec::new, |s| s.order.iter().map(|q| s.variants[q].name.clone()).collect::<Vec<_>>()),
            "asked": state.asked,
            "idle_seconds": (self.idle().as_secs_f64() * 10.0).round() / 10.0,
            "advert_break": state.in_break.as_ref().map(|b| json!({
                "roll": b.roll, "seconds": b.seconds,
                "for_seconds": (b.since.elapsed().as_secs_f64() * 10.0).round() / 10.0,
            })),
            "segments_cut": state.cut,
            "segments_passed": state.passed,
        })
    }
}
