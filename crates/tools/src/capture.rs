//! Save one master playlist and one media playlist of a live channel into `fixtures/playlists/`,
//! with everything personal or signed taken out: the viewer's address, tokens, segment paths,
//! session ids, and the channel's name.
//!
//! The tests read the fixtures that exist; add one when Twitch changes its format, then run
//! `fixtures/gen-expected.py` while the Python prototype is still in the tree.

use std::path::PathBuf;

use breakless::twitch::Twitch;
use breakless_formats::{has_ads, master_info, variants};
use clap::Args as ClapArgs;

/// Arguments of `capture`.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Goes into the file names: `NAME-master.m3u8` and `NAME-media.m3u8`.
    pub name: String,
    /// The channel. Without one: the most watched.
    pub channel: Option<String>,
    /// An HTTP proxy to ask through.
    #[arg(long)]
    pub proxy: Option<String>,
    /// Where to write. Default: `fixtures/playlists` under the current folder.
    #[arg(long, default_value = "fixtures/playlists")]
    pub out: PathBuf,
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

fn token_like(value: &str) -> bool {
    value.len() >= 40
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'=' | b'-'))
}

/// One line with `KEY="value"` pairs: the address, session ids and token-like single-letter values replaced.
fn scrub_attrs(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(eq) = rest.find("=\"") {
        let key_start = rest[..eq].rfind([',', ':']).map_or(0, |i| i + 1);
        let key = &rest[key_start..eq];
        out.push_str(&rest[..eq + 2]);
        rest = &rest[eq + 2..];
        let end = rest.find('"').unwrap_or(rest.len());
        let value = &rest[..end];
        let replaced = match key {
            "USER-IP" => "203.0.113.7",
            "SERVING-ID" | "VIDEO-SESSION-ID" | "BROADCAST-ID" => "0",
            k if k.len() == 1 && k.as_bytes()[0].is_ascii_uppercase() && token_like(value) => {
                "(removed)"
            }
            _ => value,
        };
        out.push_str(replaced);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// An address with a long path becomes `https://host/(removed)`.
fn scrub_url(line: &str) -> String {
    let Some(rest) = line.strip_prefix("https://") else {
        return line.to_string();
    };
    let Some(slash) = rest.find('/') else {
        return line.to_string();
    };
    if rest.len() - slash > 30 {
        format!("https://{}/(removed)", &rest[..slash])
    } else {
        line.to_string()
    }
}

/// The whole playlist scrubbed, the channel renamed `somechannel`.
pub fn scrub(text: &str, login: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let line = if line.starts_with('#') {
            scrub_attrs(line)
        } else {
            scrub_url(line)
        };
        out.push_str(&line.replace(login, "somechannel"));
        out.push('\n');
    }
    out
}

/// Runs the capture; returns the exit code.
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
    let login = match args.channel {
        Some(c) => c,
        None => match t.top(3).await {
            Ok(top) if !top.is_empty() => top[0].login.clone(),
            Ok(_) => {
                eprintln!("nobody is live?");
                return 1;
            }
            Err(e) => {
                eprintln!("cannot list the most-watched channels: {}", e.message);
                return 1;
            }
        },
    };
    let master = match t.master(via, &login).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{login}: {}", e.message);
            return 1;
        }
    };
    let Some(best) = variants(&master).into_iter().next() else {
        eprintln!("{login}: no qualities in the master playlist");
        return 1;
    };
    let media = match t.http(via, &best.url, None).await {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{login}: {}", e.message);
            return 1;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&args.out) {
        eprintln!("{}: {e}", args.out.display());
        return 1;
    }
    for (kind, text) in [("master", &master), ("media", &media)] {
        let path = args.out.join(format!("{}-{kind}.m3u8", args.name));
        if let Err(e) = std::fs::write(&path, scrub(text, &login.to_lowercase())) {
            eprintln!("{}: {e}", path.display());
            return 1;
        }
    }
    println!(
        "{} {login} {} {}",
        args.name,
        if has_ads(&media) { "adverts" } else { "clean" },
        master_info(&master).country
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_tokens_ids_and_paths_go() {
        let text = format!(
            "#EXT-X-TWITCH-INFO:NODE=\"n\",USER-IP=\"198.51.100.9\",SERVING-ID=\"abc\",VIDEO-SESSION-ID=\"123\",C=\"{}\",D=\"false\"\n\
             #EXT-X-STREAM-INF:BANDWIDTH=1\nhttps://usher.ttvnw.net/{}\n#EXTINF:2.000,live\nhttps://a.hls.ttvnw.net/{}\n",
            "A".repeat(50),
            "p".repeat(40),
            "q".repeat(40)
        );
        let out = scrub(&text, "someone");
        assert!(out.contains("USER-IP=\"203.0.113.7\""), "{out}");
        assert!(
            out.contains("SERVING-ID=\"0\",VIDEO-SESSION-ID=\"0\",C=\"(removed)\",D=\"false\""),
            "{out}"
        );
        assert!(
            out.contains("\nhttps://usher.ttvnw.net/(removed)\n"),
            "{out}"
        );
        assert!(
            out.contains("\nhttps://a.hls.ttvnw.net/(removed)\n"),
            "{out}"
        );
    }

    #[test]
    fn short_paths_and_the_channel_name() {
        let out = scrub(
            "#EXTINF:2.000,live\nhttps://cdn.example/someone/1.ts\n",
            "someone",
        );
        assert_eq!(
            out,
            "#EXTINF:2.000,live\nhttps://cdn.example/somechannel/1.ts\n"
        );
    }

    #[test]
    fn the_scrub_matches_the_prototype_on_a_fixture() {
        // the fixtures were scrubbed by the Python capture; scrubbing them again changes nothing
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/playlists");
        for name in ["home-clean-master.m3u8", "home-preroll-media.m3u8"] {
            let text = std::fs::read_to_string(dir.join(name)).unwrap();
            // the scrub always ends the file with one newline
            assert_eq!(
                scrub(&text, "somechannel"),
                format!("{}\n", text.trim_end_matches('\n')),
                "{name}"
            );
        }
    }
}
