//! Watch the facts other advert-cutting projects use, with the prototype's output and state.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use breakless_formats::{CLIENT_ID, PLAYER_TYPES};
use clap::Args as ClapArgs;
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(test)]
mod tests;

/// Arguments of `upstream`. The folder comes from `--dir`, then `BREAKLESS_UPSTREAM_DIR`, then
/// `upstream` relative to the current working directory.
#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Compare saved copies without downloading or writing the state file.
    #[arg(long)]
    offline: bool,
    /// Where the five source copies and upstream-state.json live.
    #[arg(long, env = "BREAKLESS_UPSTREAM_DIR", default_value = "upstream")]
    dir: PathBuf,
}

const SOURCES: &[(&str, &str)] = &[
    (
        "vaft.user.js",
        "https://raw.githubusercontent.com/ryanbr/TwitchAdSolutions/master/vaft/vaft.user.js",
    ),
    (
        "streamlink-twitch-plugin.py",
        "https://raw.githubusercontent.com/streamlink/streamlink/master/src/streamlink/plugins/twitch.py",
    ),
    (
        "ryanbr-issues.md",
        "https://raw.githubusercontent.com/ryanbr/TwitchAdSolutions/master/issues.md",
    ),
    (
        "ryanbr-full-list.md",
        "https://raw.githubusercontent.com/ryanbr/TwitchAdSolutions/master/full-list.md",
    ),
    (
        "ttv-lol-pro-README.md",
        "https://raw.githubusercontent.com/younesaassila/ttv-lol-pro/v2/README.md",
    ),
];

// Add an explanation only after the parser and its tests account for the fact, or a decision
// with a reason says that the parser needs no change.
const ACCOUNTED: &[(&str, &str)] = &[
    ("stitched-ad", "cut: AD_DATERANGE matches ID=\"stitched-ad-"),
    (
        "twitch-stitched",
        "cut: AD_DATERANGE matches the whole CLASS=\"twitch-stitched family",
    ),
    (
        "EXT-X-CUE-OUT",
        "cut: parse_media marks everything up to EXT-X-CUE-IN",
    ),
    (
        "EXT-X-DATERANGE:CLASS=\"twitch-maf-ad\"",
        "not cut, counted: an advert the official player draws itself (CLIENT_SIDE); the segments are the stream",
    ),
    (
        "EXT-X-DATERANGE:CLASS=\"twitch-trigger\"",
        "not cut: in every clean sample here (KNOWN_CLASSES); vaft uses it to fetch a backup early",
    ),
    ("/adsquared/", "cut: AD_URLS"),
    ("/_404/", "cut: AD_URLS"),
    (
        "/processing",
        "not cut: too loose a word to cut a segment on by itself; an advert under it is still caught by title or window",
    ),
    ("twitch-session", "known: KNOWN_CLASSES"),
    ("twitch-stream-source", "known: KNOWN_CLASSES"),
    ("twitch-ad-quartile", "known: KNOWN_CLASSES"),
    ("twitch-assignment", "known: KNOWN_CLASSES"),
    ("streamlink:class:twitch-stitched-ad", "cut: AD_DATERANGE"),
    ("streamlink:id:stitched-ad-", "cut: AD_DATERANGE"),
    ("streamlink:title:Amazon", "cut: parse_media's known_ad"),
];

type Facts = Vec<(&'static str, Value)>;

fn pattern(text: &str) -> Regex {
    Regex::new(text).expect("the upstream extraction patterns are valid")
}

fn first(text: &str, expression: &str) -> Option<String> {
    pattern(expression).captures(text).map(|c| c[1].to_string())
}

fn js_list(text: &str, name: &str) -> Value {
    let expression = format!(r"(?s)scope\.{}\s*=\s*\[(.*?)\];", regex::escape(name));
    let Some(body) = first(text, &expression) else {
        return Value::Null;
    };
    let body = pattern(r"//[^\n]*").replace_all(&body, "");
    let strings = pattern(r#"'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)""#)
        .captures_iter(&body)
        .map(|c| {
            c.get(1)
                .or_else(|| c.get(2))
                .map_or("", |m| m.as_str())
                .to_string()
        })
        .collect::<Vec<_>>();
    json!(strings)
}

fn streamlink_marks(text: &str) -> Value {
    let ad = first(text, r"(?s)def _is_daterange_ad\(.*?\n(.*?)\n\n");
    let segment = first(text, r"(?s)def _is_segment_ad\(.*?\n(.*?)\n\n");
    let (Some(ad), Some(segment)) = (ad, segment) else {
        return Value::Null;
    };
    let mut marks = Vec::new();
    for (body, expression, prefix) in [
        (
            ad.as_str(),
            r#"classname == "([^"]+)""#,
            "streamlink:class:",
        ),
        (ad.as_str(), r#"startswith\("([^"]+)"\)"#, "streamlink:id:"),
        (
            segment.as_str(),
            r#""([^"]+)" in title"#,
            "streamlink:title:",
        ),
    ] {
        marks.extend(
            pattern(expression)
                .captures_iter(body)
                .map(|c| format!("{prefix}{}", &c[1])),
        );
    }
    json!(marks)
}

fn facts(files: &BTreeMap<String, String>) -> Facts {
    let vaft = files.get("vaft.user.js").map_or("", String::as_str);
    let sl = files
        .get("streamlink-twitch-plugin.py")
        .map_or("", String::as_str);
    vec![
        ("vaft version", json!(first(vaft, r"@version\s+(\S+)"))),
        ("vaft advert marks", js_list(vaft, "AdSignifiers")),
        (
            "vaft classes that are not adverts",
            js_list(vaft, "KnownNonAdSignifiers"),
        ),
        (
            "vaft advert segment addresses",
            js_list(vaft, "AdSegmentURLPatterns"),
        ),
        (
            "vaft backup token kinds",
            js_list(vaft, "BackupPlayerTypes"),
        ),
        (
            "vaft client id",
            json!(first(vaft, r"scope\.ClientID\s*=\s*'([^']+)'")),
        ),
        ("streamlink advert marks", streamlink_marks(sl)),
        (
            "streamlink client id",
            json!(first(sl, r#"CLIENT_ID\s*=\s*"([a-z0-9]{20,40})""#)),
        ),
    ]
}

fn value<'a>(found: &'a Facts, name: &str) -> &'a Value {
    found
        .iter()
        .find(|(key, _)| *key == name)
        .map_or(&Value::Null, |(_, v)| v)
}

fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let non_printable = pattern(r"[\p{C}\p{Z}]");
    let mut out = String::from(quote);
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c != ' ' && non_printable.is_match(c.encode_utf8(&mut [0; 4])) => {
                let n = u32::from(c);
                if n <= 0xff {
                    write!(out, "\\x{n:02x}").expect("writing a string");
                } else if n <= 0xffff {
                    write!(out, "\\u{n:04x}").expect("writing a string");
                } else {
                    write!(out, "\\U{n:08x}").expect("writing a string");
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

fn compare(found: &Facts) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for (name, value) in found {
        if value.is_null() {
            out.push(("GONE", format!("{name}: not found where it used to be; the source was reshaped, read it and fix upstream.py")));
        }
    }
    for name in [
        "vaft advert marks",
        "vaft classes that are not adverts",
        "vaft advert segment addresses",
        "streamlink advert marks",
    ] {
        for mark in strings(value(found, name)) {
            if !ACCOUNTED.iter().any(|(known, _)| *known == mark) {
                out.push((
                    "NEW",
                    format!(
                        "{name}: {} is not accounted for in twitchlib.py",
                        python_repr(mark)
                    ),
                ));
            }
        }
    }
    for name in ["vaft client id", "streamlink client id"] {
        if let Some(client) = value(found, name).as_str().filter(|s| !s.is_empty())
            && client != CLIENT_ID
        {
            out.push(("NEW", format!("{name} is {client}, ours is {CLIENT_ID}")));
        }
    }
    for kind in strings(value(found, "vaft backup token kinds")) {
        if !PLAYER_TYPES.iter().any(|(ours, _)| *ours == kind) {
            out.push(("NEW", format!("vaft falls back to token kind {}, which probe.py does not measure (PLAYER_TYPES)", python_repr(kind))));
        }
    }
    out
}

// Python's text reader replaces invalid UTF-8 and converts CRLF and bare CR before hashing.
fn read_text(path: &Path) -> String {
    std::fs::read(path).map_or_else(
        |_| String::new(),
        |bytes| {
            String::from_utf8_lossy(&bytes)
                .replace("\r\n", "\n")
                .replace('\r', "\n")
        },
    )
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))[..16].to_string()
}

fn ascii_json(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c >= '\u{7f}' {
            for unit in c.encode_utf16(&mut [0; 2]) {
                write!(out, "\\u{unit:04x}").expect("writing a string");
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn python_json(value: &Value) -> String {
    // Pretty JSON supplies Python's spaces inside containers; remove only structural newlines.
    let pretty = serde_json::to_string_pretty(value).expect("JSON values serialize");
    let mut out = String::new();
    let mut previous = "";
    for line in pretty.lines() {
        let line = line.trim_start();
        if previous.ends_with(',') {
            out.push(' ');
        }
        out.push_str(line);
        previous = line;
    }
    ascii_json(&out)
}

fn state_json(state: &Value) -> io::Result<String> {
    let mut bytes = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, formatter);
    // Value's maps are sorted, matching sort_keys=True in the prototype.
    serde::Serialize::serialize(state, &mut serializer).map_err(io::Error::other)?;
    Ok(ascii_json(
        &String::from_utf8(bytes).map_err(io::Error::other)?,
    ))
}

fn report(
    found: &Facts,
    before: &Value,
    changed: &[&str],
    output: &mut impl Write,
) -> io::Result<i32> {
    writeln!(
        output,
        "vaft {}; {} advert marks, {} advert addresses, {} harmless classes; backup token kinds {}",
        value(found, "vaft version").as_str().unwrap_or("None"),
        strings(value(found, "vaft advert marks")).len(),
        strings(value(found, "vaft advert segment addresses")).len(),
        strings(value(found, "vaft classes that are not adverts")).len(),
        strings(value(found, "vaft backup token kinds")).join(", ")
    )?;
    for name in changed {
        writeln!(
            output,
            "changed since the last look: research/{name}  (git diff -- twitch/research/{name} shows what)"
        )?;
    }
    for (name, value) in found {
        if let Some(old) = before.get(*name)
            && old != value
        {
            writeln!(
                output,
                "was: {name} = {}\nnow: {name} = {}",
                python_json(old),
                python_json(value)
            )?;
        }
    }
    let problems = compare(found);
    for (severity, line) in &problems {
        writeln!(output, "{severity:<4} {line}")?;
    }
    Ok(i32::from(!problems.is_empty()))
}

async fn refresh(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(format!(
            "HTTP Error {}: {}",
            response.status().as_u16(),
            response.status().canonical_reason().unwrap_or("")
        )
        .into());
    }
    let bytes = response.bytes().await?;
    if bytes.len() < 500 {
        return Err(format!("only {} bytes", bytes.len()).into());
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

async fn inspect(
    args: &Args,
    sources: &[(&str, &str)],
    output: &mut impl Write,
) -> io::Result<i32> {
    std::fs::create_dir_all(&args.dir)?;
    let state_path = args.dir.join("upstream-state.json");
    let mut state: Value = std::fs::read(&state_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| json!({}));
    let client = if args.offline {
        None
    } else {
        Some(
            reqwest::Client::builder()
                .user_agent("breakless-upstream")
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(io::Error::other)?,
        )
    };
    let mut files = BTreeMap::new();
    let mut changed = Vec::new();
    for &(name, url) in sources {
        let path = args.dir.join(name);
        if let Some(client) = &client
            && let Err(error) = refresh(client, url, &path).await
        {
            writeln!(
                output,
                "could not fetch {name}: {error} (using the copy in upstream/)"
            )?;
        }
        let text = read_text(&path);
        let digest = digest(&text);
        let state_files = state
            .as_object_mut()
            .ok_or_else(|| io::Error::other("state is not an object"))?
            .entry("files")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| io::Error::other("state files is not an object"))?;
        if let Some(old) = state_files.get(name).filter(|old| !old.is_null())
            && old != &digest
        {
            changed.push(name);
        }
        state_files.insert(name.to_string(), json!(digest));
        files.insert(name.to_string(), text);
    }
    let found = facts(&files);
    let code = report(&found, &state["facts"], &changed, output)?;
    if !args.offline {
        state["facts"] = found
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        state["looked"] = json!(jiff::Zoned::now().strftime("%Y-%m-%d %H:%M").to_string());
        std::fs::write(state_path, state_json(&state)?)?;
    }
    if code == 0 {
        writeln!(
            output,
            "nothing new: everything the others look for is accounted for in twitchlib.py"
        )?;
    }
    Ok(code)
}

/// Runs the watcher; returns 1 when a fact needs a look or a local file cannot be written.
pub async fn run(args: Args) -> i32 {
    match inspect(&args, SOURCES, &mut io::stdout()).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}
