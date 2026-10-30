//! Watch several channels through the service for a while, the way players would, and report
//! every advert break it met. This is how mid-roll adverts are measured (`probe` only sees
//! pre-rolls).
//!
//! Each channel is polled every 2 s. Only playlists are fetched (a few kB), no video. Columns:
//! `from`/`seen` are where its playlists are asked from now and the country Twitch sees;
//! `passed`/`cut` are stream segments handed on and advert segments cut; `stalled_s` is seconds in
//! which the playlist did not move for more than 10 s at a stretch (a frozen picture to a viewer);
//! `errors` are requests that failed (a channel that went offline shows many). The breaks
//! themselves, with times, are in the service's `adverts.jsonl`.

use std::time::{Duration, Instant};

use clap::Args as ClapArgs;
use serde_json::Value;

/// Arguments of `soak`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// The service to watch through.
    #[arg(
        short = 'b',
        long,
        env = "TWITCH_BASE",
        default_value = "http://127.0.0.1:8080"
    )]
    pub base: String,
    /// How long, in minutes.
    #[arg(short = 'm', long, default_value_t = 20.0)]
    pub minutes: f64,
    /// How many of the most-watched channels (when none are named).
    #[arg(short = 'n', long, default_value_t = 6)]
    pub count: usize,
    /// Channel names. Without any: the most watched, as the service lists them.
    pub channels: Vec<String>,
}

struct Watch {
    channel: String,
    stalled: f64,
    errors: u64,
}

async fn get(client: &reqwest::Client, url: &str) -> Result<String, reqwest::Error> {
    client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await
}

async fn watch(client: reqwest::Client, base: String, channel: String, end: Instant) -> Watch {
    let mut newest: Option<String> = None;
    let mut since = Instant::now();
    let mut out = Watch {
        channel: channel.clone(),
        stalled: 0.0,
        errors: 0,
    };
    while Instant::now() < end {
        match get(&client, &format!("{base}/live/{channel}/chunked.m3u8")).await {
            Ok(text) => {
                let tail = text
                    .trim_end()
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if newest.as_deref() != Some(tail.as_str()) {
                    if newest.is_some() && since.elapsed() > Duration::from_secs(10) {
                        out.stalled += since.elapsed().as_secs_f64();
                    }
                    newest = Some(tail);
                    since = Instant::now();
                }
            }
            Err(_) => out.errors += 1,
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if since.elapsed() > Duration::from_secs(10) {
        out.stalled += since.elapsed().as_secs_f64();
    }
    out
}

/// Runs the soak; returns the exit code.
pub async fn run(args: Args) -> i32 {
    let base = args.base.trim_end_matches('/').to_string();
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cannot build the HTTP client: {e}");
            return 2;
        }
    };
    let channels: Vec<String> = if args.channels.is_empty() {
        let list = match get(&client, &format!("{base}/api/channels")).await {
            Ok(text) => text,
            Err(e) => {
                eprintln!("{base}/api/channels: {e}");
                return 2;
            }
        };
        let list: Value = serde_json::from_str(&list).unwrap_or(Value::Null);
        list["top"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["login"].as_str().map(str::to_string))
            .take(args.count)
            .collect()
    } else {
        args.channels.clone()
    };
    if channels.is_empty() {
        eprintln!("no channels to watch");
        return 2;
    }
    let end = Instant::now() + Duration::from_secs_f64(args.minutes * 60.0);
    let mut tasks = Vec::new();
    for c in &channels {
        tasks.push(tokio::spawn(watch(
            client.clone(),
            base.clone(),
            c.clone(),
            end,
        )));
    }
    let mut results = Vec::new();
    for t in tasks {
        if let Ok(w) = t.await {
            results.push(w);
        }
    }
    let status: Value = get(&client, &format!("{base}/api/status"))
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    println!(
        "{:<20} {:<5} {:<4} {:>8} {:>6} {:>9} {:>6}",
        "channel", "from", "seen", "passed", "cut", "stalled_s", "errors"
    );
    for w in &results {
        let mine = status["watching"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|x| x["channel"].as_str() == Some(w.channel.to_lowercase().as_str()));
        let field = |k: &str| {
            mine.map_or("None".to_string(), |m| match &m[k] {
                Value::Null => "None".to_string(),
                Value::String(s) => s.clone(),
                v => v.to_string(),
            })
        };
        println!(
            "{:<20} {:<5} {:<4} {:>8} {:>6} {:>9.0} {:>6}",
            w.channel.chars().take(19).collect::<String>(),
            field("from"),
            field("country"),
            field("segments_passed"),
            field("segments_cut"),
            w.stalled,
            w.errors
        );
    }
    println!("{}", status["totals"]);
    0
}
