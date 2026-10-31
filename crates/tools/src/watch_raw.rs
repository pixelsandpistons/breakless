//! Follow one channel's playlist exactly as Twitch sends it and print a line whenever the kinds of
//! tag or the segment titles change. This is how to see what Twitch is really doing when the
//! service seems to cut too much or too little.
//!
//! Reading it: titles `live` is stream. `Amazon` or `DCM` are advert segments (class
//! `twitch-stitched-ad` comes with them): those are cut. Class `twitch-maf-ad` with titles still
//! `live` is Twitch asking its own player to show an advert; the video is untouched and nothing is
//! cut. Anything else is new: capture it (`capture`) and teach the parser about it.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use breakless::twitch::Twitch;
use breakless_formats::attrs::attr;
use breakless_formats::{master_info, parse_media, variants};
use clap::Args as ClapArgs;

const BORING: &[&str] = &[
    "timestamp",
    "twitch-session",
    "twitch-stream-source",
    "twitch-trigger",
];

/// Arguments of `watch-raw`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// The channel to follow.
    pub channel: String,
    /// How long, in minutes.
    #[arg(short = 'm', long, default_value_t = 5.0)]
    pub minutes: f64,
    /// An HTTP proxy to ask through.
    #[arg(long)]
    pub proxy: Option<String>,
    /// Also print each new advert-ish DATERANGE line in full (long values shortened).
    #[arg(long)]
    pub dump: bool,
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
}

/// The DATERANGE classes (minus the boring ones) and the title prefixes in a playlist.
fn shape(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut classes = BTreeSet::new();
    let mut titles = BTreeSet::new();
    for line in text.lines() {
        if line.starts_with("#EXT-X-DATERANGE:") {
            if let Some(c) = attr(line, "CLASS")
                && !BORING.contains(&c)
            {
                classes.insert(c.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let title = rest.split_once(',').map_or("", |(_, t)| t);
            titles.insert(title.split('|').next().unwrap_or("").to_string());
        }
    }
    (classes, titles)
}

/// A DATERANGE line with any value over 80 characters replaced by `(long)`.
fn shorten(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find("=\"") {
        out.push_str(&rest[..start + 2]);
        rest = &rest[start + 2..];
        let end = rest.find('"').unwrap_or(rest.len());
        if end >= 80 {
            out.push_str("(long)");
        } else {
            out.push_str(&rest[..end]);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// Runs the watch; returns the exit code.
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
    let master = match t.master(via, &args.channel).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{}: {}", args.channel, e.message);
            return 1;
        }
    };
    let Some(best) = variants(&master).into_iter().next() else {
        eprintln!("{}: no qualities in the master playlist", args.channel);
        return 1;
    };
    println!(
        "Twitch sees us in {}; watching {}",
        master_info(&master).country,
        args.channel
    );
    let start = Instant::now();
    let end = start + Duration::from_secs_f64(args.minutes * 60.0);
    let mut seen: Option<(BTreeSet<String>, BTreeSet<String>)> = None;
    let mut shown: BTreeSet<String> = BTreeSet::new();
    while Instant::now() < end {
        let text = match t.http(via, &best.url, None).await {
            Ok(text) => text,
            Err(e) => {
                println!("t={:4}s  error: {}", start.elapsed().as_secs(), e.message);
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let now = shape(&text);
        if seen.as_ref() != Some(&now) {
            let media = parse_media(&text);
            let cut = media.segments.iter().filter(|s| s.ad).count();
            println!(
                "t={:4}s  classes={}  titles={:?}  we would cut {cut} of {} segments",
                start.elapsed().as_secs(),
                if now.0.is_empty() {
                    "none".to_string()
                } else {
                    now.0.iter().cloned().collect::<Vec<_>>().join(",")
                },
                now.1.iter().cloned().collect::<Vec<_>>(),
                media.segments.len()
            );
            seen = Some(now);
        }
        if args.dump {
            for line in text.lines().filter(|l| l.starts_with("#EXT-X-DATERANGE:")) {
                if let Some(id) = attr(line, "ID")
                    && (id.starts_with("stitched-ad") || id.starts_with("maf-ad"))
                    && shown.insert(id.to_string())
                {
                    println!("   {}", shorten(line));
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_is_classes_and_title_prefixes() {
        let text = "#EXT-X-DATERANGE:ID=\"a\",CLASS=\"twitch-session\"\n#EXT-X-DATERANGE:ID=\"b\",CLASS=\"twitch-stitched-ad\"\n\
                    #EXTINF:2.000,live\nhttps://x/1.ts\n#EXTINF:2.000,Amazon|123\nhttps://x/2.ts\n";
        let (classes, titles) = shape(text);
        assert_eq!(
            classes.into_iter().collect::<Vec<_>>(),
            ["twitch-stitched-ad"]
        );
        assert_eq!(titles.into_iter().collect::<Vec<_>>(), ["Amazon", "live"]);
    }

    #[test]
    fn long_values_are_shortened() {
        let line = format!(
            "#EXT-X-DATERANGE:ID=\"stitched-ad-1\",X-TV-TWITCH-AD-ADVERIFICATIONS=\"{}\",DURATION=29.297",
            "e".repeat(120)
        );
        assert_eq!(
            shorten(&line),
            "#EXT-X-DATERANGE:ID=\"stitched-ad-1\",X-TV-TWITCH-AD-ADVERIFICATIONS=\"(long)\",DURATION=29.297"
        );
    }
}
