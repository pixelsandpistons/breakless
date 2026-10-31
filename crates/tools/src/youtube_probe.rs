//! Measure live HLS access and playlist observations before deciding on service support.
//!
//! The probe uses no cookies or proof-of-origin tokens. Discovery searches for `live`, selecting
//! only results marked live; the chosen client then supplies each player request. Only `--capture`
//! writes files. Playlist observations are evidence for review, not advert classifications.

mod client;
mod discovery;
mod playlist;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use breakless_formats::{Variant, variants};
use clap::Args as ClapArgs;
use reqwest::{Client as Http, Url};
use serde_json::Value;
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior};

use client::Client;
use playlist::{Counts, scrub};

const API: &str = "https://www.youtube.com/youtubei/v1/";

/// Arguments of `youtube-probe`, a measurement tool for the maintainer's home line.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Innertube player client. Its current identity is copied from yt-dlp.
    #[arg(long, value_enum, default_value = "web_safari")]
    client: Client,
    /// Live videos to find when no video IDs are supplied.
    #[arg(short = 'n', default_value_t = 12, value_parser = clap::value_parser!(u32).range(1..))]
    count: u32,
    /// Minutes to poll each best media playlist every two seconds; zero takes one snapshot.
    #[arg(long, default_value = "5", value_parser = minutes)]
    minutes: Duration,
    /// Save scrubbed master and media snapshots in this folder. Without it, write no files.
    #[arg(long)]
    capture: Option<PathBuf>,
    /// Video IDs. Without IDs, search for `live` and select results marked live.
    #[arg(value_parser = video_id)]
    ids: Vec<String>,
}

fn minutes(value: &str) -> Result<Duration, String> {
    let value = value
        .parse::<f64>()
        .map_err(|_| "minutes must be a number")?;
    Duration::try_from_secs_f64(value * 60.0)
        .map_err(|_| "minutes must be finite and nonnegative".to_string())
}

fn video_id(value: &str) -> Result<String, String> {
    if value.len() == 11
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        Ok(value.to_string())
    } else {
        Err("video IDs must contain 11 letters, digits, underscores or hyphens".to_string())
    }
}

fn http(client: Client) -> Result<Http, reqwest::Error> {
    Http::builder()
        .no_proxy()
        .default_headers(client.headers())
        .timeout(Duration::from_secs(10))
        .build()
}

async fn fetch(
    http: &Http,
    url: &Url,
    body: Option<&Value>,
    stage: &str,
) -> Result<String, String> {
    let request = match body {
        Some(body) => http
            .post(url.clone())
            .body(serde_json::to_vec(body).map_err(|_| format!("{stage}: cannot encode request"))?),
        None => http.get(url.clone()),
    };
    let response = request.send().await.map_err(|error| {
        let reason = if error.is_timeout() {
            "timeout"
        } else if error.is_connect() {
            "connection failed"
        } else {
            "request failed"
        };
        format!("{stage}: {reason}")
    })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| format!("{stage}: response body failed"))?;
    if !status.is_success() {
        return Err(format!(
            "{stage}: HTTP {}{}",
            status.as_u16(),
            if bot_check(&text) { " (bot check)" } else { "" }
        ));
    }
    Ok(text)
}

async fn api(http: &Http, url: &Url, body: &Value, stage: &str) -> Result<Value, String> {
    let text = fetch(http, url, Some(body), stage).await?;
    serde_json::from_str(&text).map_err(|_| {
        if bot_check(&text) {
            format!("{stage}: bot check")
        } else {
            format!("{stage}: not JSON")
        }
    })
}

fn bot_check(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "bot check",
        "not a bot",
        "unusual traffic",
        "confirm you're",
        "confirm you’re",
        "confirm you are",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

async fn discover(http: &Http, url: &Url, wanted: usize) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    let mut seen = BTreeSet::new();
    let mut tokens = BTreeSet::new();
    let mut continuation = None;
    for _ in 0..20 {
        let response = api(
            http,
            url,
            &discovery::search_body(continuation.as_deref()),
            "search",
        )
        .await?;
        for id in discovery::live_ids(&response) {
            if seen.insert(id.clone()) {
                ids.push(id);
            }
            if ids.len() == wanted {
                return Ok(ids);
            }
        }
        continuation = discovery::continuation(&response);
        if !continuation
            .as_ref()
            .is_some_and(|token| tokens.insert(token.clone()))
        {
            break;
        }
    }
    Ok(ids)
}

struct Row {
    id: String,
    answered: bool,
    status: String,
    hls: bool,
    variants: Vec<Variant>,
    counts: Counts,
    polls: u64,
    bot_check: bool,
    errors: Vec<String>,
}

impl Row {
    fn new(id: String) -> Self {
        Self {
            id,
            answered: false,
            status: "NO_RESPONSE".to_string(),
            hls: false,
            variants: Vec::new(),
            counts: Counts::default(),
            polls: 0,
            bot_check: false,
            errors: Vec::new(),
        }
    }

    fn login_required(&self) -> bool {
        self.status == "LOGIN_REQUIRED"
    }

    fn observations(&self) -> bool {
        self.counts.discontinuities > 0
            || !self.counts.classes.is_empty()
            || self.counts.hosts.len() > 1
    }

    fn error(&mut self, error: String) {
        self.bot_check |= bot_check(&error);
        self.errors.push(error);
    }

    fn player(&mut self, response: &Value) {
        self.answered = true;
        self.status = response["playabilityStatus"]["status"]
            .as_str()
            .unwrap_or("MISSING_STATUS")
            .to_string();
        self.bot_check = bot_check(&response["playabilityStatus"].to_string());
        if self.status != "OK" {
            self.error(format!("player: {}", self.status));
        }
    }
}

fn manifest(text: &str, stage: &str) -> Result<(), String> {
    if text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("#EXTM3U")
    {
        Ok(())
    } else if bot_check(text) {
        Err(format!("{stage}: bot check"))
    } else {
        Err(format!("{stage}: not an HLS playlist"))
    }
}

fn media_address(master: &str, base: &Url) -> Result<(Vec<Variant>, Url), String> {
    let mut qualities = variants(master);
    for quality in &mut qualities {
        quality.url = base
            .join(&quality.url)
            .map_err(|_| "master: invalid variant URL")?
            .to_string();
    }
    let media = match qualities.first() {
        Some(best) => Url::parse(&best.url).map_err(|_| "master: invalid variant URL")?,
        None if master.lines().any(|line| line.starts_with("#EXTINF:")) => base.clone(),
        None => return Err("master: no variants or media segments".to_string()),
    };
    Ok((qualities, media))
}

struct Snapshots {
    master: String,
    media: Option<String>,
    identities: Vec<String>,
}

fn capture(dir: &Path, client: Client, number: usize, snapshots: &Snapshots) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|_| "capture: cannot create folder")?;
    let identities: Vec<_> = snapshots.identities.iter().map(String::as_str).collect();
    for (kind, text) in [
        ("master", Some(&snapshots.master)),
        ("media", snapshots.media.as_ref()),
    ] {
        if let Some(text) = text {
            let path = dir.join(format!("youtube-{}-{number}-{kind}.m3u8", client.name()));
            std::fs::write(path, scrub(text, &identities))
                .map_err(|_| "capture: cannot write playlist")?;
        }
    }
    Ok(())
}

async fn poll(
    http: &Http,
    url: &Url,
    duration: Duration,
    initial: Option<&str>,
    row: &mut Row,
) -> Option<String> {
    let Some(deadline) = Instant::now().checked_add(duration) else {
        row.error("media: duration is too large".to_string());
        return None;
    };
    let mut ticks = tokio::time::interval(Duration::from_secs(2));
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut snapshot = None;
    loop {
        if row.polls > 0 && Instant::now() >= deadline {
            break;
        }
        tokio::select! {
            () = tokio::time::sleep_until(deadline), if row.polls > 0 => break,
            _ = ticks.tick() => {}
        }
        let pending = async {
            if row.polls == 0
                && let Some(initial) = initial
            {
                Ok(initial.to_string())
            } else {
                fetch(http, url, None, "media").await
            }
        };
        let response = if duration.is_zero() {
            pending.await
        } else {
            tokio::select! {
                response = pending => response,
                () = tokio::time::sleep_until(deadline) => {
                    if row.polls == 0 { row.error("media: no playlist before deadline".to_string()); }
                    break;
                }
            }
        };
        row.polls += 1;
        match response {
            Ok(text) => match manifest(&text, "media") {
                Ok(()) => {
                    let before = (
                        row.counts.discontinuities,
                        row.counts.classes.len(),
                        row.counts.hosts.len(),
                    );
                    row.counts.feed(&text, url);
                    let after = (
                        row.counts.discontinuities,
                        row.counts.classes.len(),
                        row.counts.hosts.len(),
                    );
                    if snapshot.is_none() || before != after {
                        snapshot = Some(text);
                    }
                }
                Err(error) => {
                    row.bot_check |= bot_check(&error);
                    row.error(error);
                }
            },
            Err(error) => row.error(error),
        }
        if duration.is_zero() {
            break;
        }
    }
    snapshot
}

async fn measure(
    http: &Http,
    player_url: &Url,
    client: Client,
    id: String,
    duration: Duration,
) -> (Row, Option<Snapshots>) {
    let mut row = Row::new(id);
    let response = match api(http, player_url, &client.player_body(&row.id), "player").await {
        Ok(response) => response,
        Err(error) => {
            row.bot_check = bot_check(&error);
            row.error(error);
            return (row, None);
        }
    };
    row.player(&response);
    let Some(master_url) = response["streamingData"]["hlsManifestUrl"]
        .as_str()
        .filter(|s| !s.is_empty())
    else {
        return (row, None);
    };
    row.hls = true;
    let Ok(master_url) = Url::parse(master_url) else {
        row.error("master: invalid URL".to_string());
        return (row, None);
    };
    let master = match fetch(http, &master_url, None, "master").await {
        Ok(master) => master,
        Err(error) => {
            row.error(error);
            return (row, None);
        }
    };
    if let Err(error) = manifest(&master, "master") {
        row.bot_check |= bot_check(&error);
        row.error(error);
        return (row, None);
    }
    let mut snapshots = Snapshots {
        master,
        media: None,
        identities: vec![row.id.clone()],
    };
    for key in ["author", "channelId"] {
        if let Some(value) = response["videoDetails"][key].as_str() {
            snapshots.identities.push(value.to_string());
        }
    }
    match media_address(&snapshots.master, &master_url) {
        Ok((qualities, media)) => {
            let initial = if qualities.is_empty() {
                Some(snapshots.master.as_str())
            } else {
                None
            };
            row.variants = qualities;
            snapshots.media = poll(http, &media, duration, initial, &mut row).await;
        }
        Err(error) => row.error(error),
    }
    (row, Some(snapshots))
}

fn print_table(client: Client, rows: &[Row]) {
    println!(
        "{:<12} {:<18} {:<3} {:>8} {:>5} {:>8} {:>7} {:>6}",
        "video", "playabilityStatus", "HLS", "variants", "polls", "segments", "discont", "errors"
    );
    for row in rows {
        println!(
            "{:<12} {:<18} {:<3} {:>8} {:>5} {:>8} {:>7} {:>6}",
            row.id,
            row.status,
            if row.hls { "yes" } else { "no" },
            row.variants.len(),
            row.polls,
            row.counts.segments,
            row.counts.discontinuities,
            row.errors.len()
        );
        let qualities: Vec<_> = row
            .variants
            .iter()
            .map(|v| {
                format!(
                    "{} {}bit/s {} {}fps",
                    v.slug, v.bandwidth, v.resolution, v.fps
                )
            })
            .collect();
        println!(
            "  {} variants: {}",
            row.id,
            if qualities.is_empty() {
                "(none)".to_string()
            } else {
                qualities.join("; ")
            }
        );
        println!(
            "  {} DATERANGE classes: {}",
            row.id,
            words(&row.counts.classes)
        );
        println!("  {} segment hosts: {}", row.id, words(&row.counts.hosts));
        println!(
            "  {} LOGIN_REQUIRED={} bot_check={}",
            row.id,
            row.login_required(),
            row.bot_check
        );
        let mut errors = BTreeMap::new();
        for error in &row.errors {
            *errors.entry(error).or_insert(0_u64) += 1;
        }
        for (error, count) in errors {
            println!("  {} {error} ({count} occurrences)", row.id);
        }
    }
    println!(
        "{}: answered {} / with HLS {} / with anything besides content {} / errors {}",
        client.name(),
        rows.iter().filter(|r| r.answered).count(),
        rows.iter().filter(|r| r.hls).count(),
        rows.iter().filter(|r| r.observations()).count(),
        rows.iter().map(|r| r.errors.len()).sum::<usize>()
    );
}

fn words(values: &BTreeSet<String>) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

/// Runs the measurement; returns 0 when requests complete without recorded errors, 1 for measured
/// errors, or 2 when setup or discovery prevents measurement. A missing HLS URL alone is a result.
pub async fn run(args: Args) -> i32 {
    let Ok(http) = http(args.client) else {
        eprintln!("cannot build HTTP client");
        return 2;
    };
    let api_root = Url::parse(API).expect("the Innertube API address parses");
    let ids = if args.ids.is_empty() {
        let Ok(search_http) = self::http(Client::WebSafari) else {
            eprintln!("cannot build search HTTP client");
            return 2;
        };
        match discover(
            &search_http,
            &api_root
                .join("search?prettyPrint=false")
                .expect("search address"),
            args.count as usize,
        )
        .await
        {
            Ok(ids) if !ids.is_empty() => {
                println!(
                    "discovery: search for live with web_safari; selected {} of {} requested live-marked videos",
                    ids.len(),
                    args.count
                );
                ids
            }
            Ok(_) => {
                eprintln!("search: no live-marked videos found");
                return 2;
            }
            Err(error) => {
                eprintln!("{error}");
                return 2;
            }
        }
    } else {
        args.ids
    };
    let player_url = api_root
        .join("player?prettyPrint=false")
        .expect("player address");
    let mut jobs = JoinSet::new();
    for (index, id) in ids.into_iter().enumerate() {
        let http = http.clone();
        let player_url = player_url.clone();
        let capture_dir = args.capture.clone();
        jobs.spawn(async move {
            let (mut row, snapshots) =
                measure(&http, &player_url, args.client, id, args.minutes).await;
            if let (Some(dir), Some(snapshots)) = (capture_dir, snapshots)
                && let Err(error) = capture(&dir, args.client, index + 1, &snapshots)
            {
                row.error(error);
            }
            (index, row)
        });
    }
    let mut rows = Vec::new();
    while let Some(result) = jobs.join_next().await {
        let Ok(row) = result else {
            eprintln!("measurement task failed");
            return 2;
        };
        rows.push(row);
    }
    rows.sort_by_key(|(index, _)| *index);
    let rows: Vec<_> = rows.into_iter().map(|(_, row)| row).collect();
    print_table(args.client, &rows);
    i32::from(rows.iter().any(|row| !row.errors.is_empty()))
}
