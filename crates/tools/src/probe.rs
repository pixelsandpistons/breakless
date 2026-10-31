//! Measure where Twitch serves adverts right now.
//!
//! A fresh token is what a viewer gets on opening a stream, so this measures pre-roll adverts.
//! Mid-roll adverts only show while watching; the service counts those itself. The last lines,
//! `clean / answered` and `Twitch saw us in`, are what the estate's health check reads; their shape
//! is the prototype's (`reference/python/probe.py`).

use std::collections::{BTreeMap, BTreeSet};

use breakless::twitch::{Twitch, random_u64};
use breakless_formats::{PLAYER_TYPES, master_info, parse_media, variants};
use clap::Args as ClapArgs;
use serde_json::{Value, json};

/// Arguments of `probe`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// How many of the most-watched channels to ask (when none are named).
    #[arg(short = 'n', default_value_t = 6)]
    pub count: usize,
    /// Every token kind, not only `site`.
    #[arg(long)]
    pub types: bool,
    /// Pick the channels at random from the 30 most watched, so repeated runs do not keep asking
    /// the same ones (Twitch shows a pre-roll once, then not again for some minutes).
    #[arg(long)]
    pub random: bool,
    /// An HTTP proxy to ask through.
    #[arg(long)]
    pub proxy: Option<String>,
    /// The same as data.
    #[arg(long)]
    pub json: bool,
    /// Twitch's GraphQL endpoint. Only tests change it.
    #[arg(
        long,
        env = "BREAKLESS_GQL_URL",
        default_value = "https://gql.twitch.tv/gql",
        hide = true
    )]
    pub gql_url: String,
    /// Where live playlists come from. Only tests change it.
    #[arg(
        long,
        env = "BREAKLESS_USHER_URL",
        default_value = "https://usher.ttvnw.net/api/channel/hls/{login}.m3u8",
        hide = true
    )]
    pub usher_url: String,
    /// Channel names. Without any: the most watched.
    pub channels: Vec<String>,
}

async fn probe_one(
    t: &Twitch,
    via: &breakless::twitch::Via,
    login: &str,
    kinds: &[(&str, &str)],
) -> BTreeMap<String, Value> {
    let mut row = BTreeMap::new();
    for (kind, platform) in kinds {
        let result = async {
            let (value, signature) = t.token(via, login, kind, platform).await?;
            let master = t
                .http(via, &t.master_url(login, &value, &signature), None)
                .await?;
            let v = variants(&master);
            let Some(best) = v.first() else {
                return Ok::<Value, breakless::twitch::Error>(json!({"error": "no variants"}));
            };
            let media = parse_media(&t.http(via, &best.url, None).await?);
            let info = master_info(&master);
            Ok(json!({
                "ads": media.ads.is_some(),
                "roll": media.ads.as_ref().map_or("", |a| a.roll.as_str()),
                "seconds": media.ads.as_ref().map_or("", |a| a.seconds.as_str()),
                "best": best.name, "qualities": v.len(),
                "country": info.country, "cluster": info.cluster,
                "odd_titles": media.odd_titles,
            }))
        }
        .await;
        row.insert(
            (*kind).to_string(),
            result
                .unwrap_or_else(|e| json!({"error": breakless::twitch::truncate(&e.message, 120)})),
        );
    }
    row
}

/// Runs the probe; returns the exit code.
pub async fn run(args: Args) -> i32 {
    let proxies: Vec<String> = args.proxy.iter().cloned().collect();
    let t = match Twitch::new(
        &proxies,
        &args.gql_url,
        &args.usher_url,
        breakless::twitch::EXIT_REST,
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot build the HTTP client: {e}");
            return 2;
        }
    };
    let via = t.via(if proxies.is_empty() { None } else { Some(0) });
    let kinds: Vec<(&str, &str)> = if args.types {
        PLAYER_TYPES.to_vec()
    } else {
        PLAYER_TYPES[..1].to_vec()
    };
    let logins: Vec<String> = if args.channels.is_empty() {
        let top = match t
            .top(if args.random { 30 } else { args.count as u32 })
            .await
        {
            Ok(top) => top,
            Err(e) => {
                eprintln!("cannot list the most-watched channels: {}", e.message);
                return 2;
            }
        };
        let mut logins: Vec<String> = top.into_iter().map(|c| c.login).collect();
        if args.random {
            // a Fisher-Yates shuffle with the standard library's randomness, then the first n
            for i in (1..logins.len()).rev() {
                logins.swap(i, (random_u64() % (i as u64 + 1)) as usize);
            }
            logins.truncate(args.count);
        }
        logins
    } else {
        args.channels.clone()
    };
    let mut channels: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut order = Vec::new();
    for login in &logins {
        channels.insert(login.clone(), probe_one(&t, via, login, &kinds).await);
        order.push(login.clone());
    }
    if args.json {
        println!(
            "{}",
            json!({"when": breakless::twitch::now_text(), "proxy": !proxies.is_empty(), "channels": channels})
        );
        return 0;
    }
    print_table(&order, &channels, &kinds);
    0
}

/// The table the prototype prints; its last lines are what `check.sh` reads.
fn print_table(
    order: &[String],
    channels: &BTreeMap<String, BTreeMap<String, Value>>,
    kinds: &[(&str, &str)],
) {
    let names: Vec<&str> = kinds.iter().map(|(k, _)| *k).collect();
    print!("{:<20}", "channel");
    for n in &names {
        print!("{n:<22}");
    }
    println!();
    let mut tally: BTreeMap<&str, (u32, u32)> = names.iter().map(|n| (*n, (0, 0))).collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut errors: BTreeSet<String> = BTreeSet::new();
    let mut odd: BTreeSet<String> = BTreeSet::new();
    for login in order {
        let row = &channels[login];
        print!("{:<20}", login.chars().take(19).collect::<String>());
        for n in &names {
            let r = &row[*n];
            if let Some(e) = r.get("error") {
                errors.insert(format!("  error {n}: {}", e.as_str().unwrap_or("")));
                print!("{:<22}", "error");
                continue;
            }
            let entry = tally.get_mut(n).expect("every kind is tallied");
            entry.1 += 1;
            let ads = r["ads"].as_bool().unwrap_or(false);
            if !ads {
                entry.0 += 1;
            }
            seen.insert(format!(
                "{} ({})",
                r["country"].as_str().unwrap_or(""),
                r["cluster"].as_str().unwrap_or("")
            ));
            for title in r["odd_titles"].as_array().into_iter().flatten() {
                odd.insert(title.as_str().unwrap_or("").to_string());
            }
            let best = r["best"]
                .as_str()
                .unwrap_or("")
                .split(' ')
                .next()
                .unwrap_or("")
                .to_string();
            let cell = if ads {
                format!(
                    "ADS {} {}s {best}",
                    r["roll"].as_str().unwrap_or(""),
                    r["seconds"].as_str().unwrap_or("")
                )
            } else {
                format!("clean {best}")
            };
            print!("{:<22}", cell.chars().take(21).collect::<String>());
        }
        println!();
    }
    print!("{:<20}", "clean / answered");
    for n in &names {
        let (clean, answered) = tally[n];
        print!("{:<22}", format!("{clean}/{answered}"));
    }
    println!();
    println!(
        "Twitch saw us in: {}",
        seen.into_iter().collect::<Vec<_>>().join(", ")
    );
    for e in errors {
        println!("{e}");
    }
    if !odd.is_empty() {
        println!(
            "  segment titles not seen before (STREAM_TITLES): {}",
            odd.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
}
