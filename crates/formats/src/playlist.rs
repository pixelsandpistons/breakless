//! Twitch's master and media playlists: the qualities offered, the facts Twitch states about the
//! viewer, and which segments are adverts.
//!
//! What marks an advert in a media playlist (seen live 2026-10-04; the same evidence vaft and
//! Streamlink use):
//!
//! - a `#EXT-X-DATERANGE` whose class starts `twitch-stitched` or whose id starts `stitched-ad-`,
//!   with a start and a duration: every segment inside that window is an advert;
//! - the segment's title: stream segments are titled `live`, adverts `Amazon|id` or `DCM|id`;
//! - a segment address containing one of [`AD_URLS`];
//! - everything between `#EXT-X-CUE-OUT` and `#EXT-X-CUE-IN`.
//!
//! Not an advert in the video: a `#EXT-X-DATERANGE` of class `twitch-maf-ad`. It appears a few
//! minutes into every session, from any country, and asks Twitch's own player to draw an advert over
//! the stream (`"is_stitched": false` inside it). The segments under it are the stream. Cutting on it
//! once threw away a minute of real stream per session; it is counted ([`Media::client_side`]) and
//! otherwise ignored.
//!
//! Anything the parser has not met before (a segment title that is neither `live` nor an advert
//! shape, a `DATERANGE` class in neither list) is reported in [`Media::odd_titles`] and
//! [`Media::odd_classes`] and never cut, so that somebody looks before it costs a stream.

use std::collections::BTreeSet;

use crate::attrs::{attr, when};

/// The public web player's client id. Every third-party player uses it; it is not a secret.
pub const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// Token kinds as `(playerType, platform)`. The service uses the first; the probe measures all.
pub const PLAYER_TYPES: &[(&str, &str)] = &[
    ("site", "web"),
    ("popout", "web"),
    ("embed", "web"),
    ("mobile_web", "web"),
    ("autoplay", "android"),
    ("picture-by-picture", "web"),
    ("thunderdome", "web"),
    ("frontpage", "web"),
];

/// The GraphQL query for a live channel's playback token.
pub const TOKEN_QUERY: &str = r#"query PlaybackAccessToken($login: String!, $playerType: String!, $platform: String!) {
  streamPlaybackAccessToken(channelName: $login, params: {platform: $platform, playerBackend: "mediaplayer", playerType: $playerType}) {
    value
    signature
  }
}"#;

/// Segment addresses that are adverts whatever their title (from vaft).
pub const AD_URLS: &[&str] = &["/adsquared/", "/_404/"];

/// Titles of stream segments. Anything else that is not a known advert shape is reported.
pub const STREAM_TITLES: &[&str] = &["live", ""];

/// `DATERANGE` classes met so far that say nothing about adverts in the video.
pub const KNOWN_CLASSES: &[&str] = &[
    "timestamp",
    "twitch-session",
    "twitch-stream-source",
    "twitch-trigger",
    "twitch-ad-quartile",
    "twitch-assignment",
    "twitch-maf-ad",
];

const CLIENT_SIDE: &str = r#"CLASS="twitch-maf-ad""#;

/// One quality in a master playlist, as Twitch offers it.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    /// Twitch's group id: `chunked` is the source quality, then `720p60`, `480p30`, `audio_only`.
    /// Stable from one session to the next, which the names are not.
    pub slug: String,
    /// The name shown to people (`1080p60 (source)`).
    pub name: String,
    /// Bits per second.
    pub bandwidth: u64,
    /// `1920x1080`, or empty for audio.
    pub resolution: String,
    /// `60.000`, or empty.
    pub fps: String,
    /// `avc1.64002A,mp4a.40.2`, or empty.
    pub codecs: String,
    /// The media playlist's address.
    pub url: String,
}

/// What Twitch states about the viewer in a master playlist's `#EXT-X-TWITCH-INFO` line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MasterInfo {
    /// Two letters, the country Twitch placed the request in.
    pub country: String,
    /// The CDN cluster serving the video.
    pub cluster: String,
    /// The edge node.
    pub node: String,
    /// Twitch's own id for this viewing session.
    pub session: String,
}

/// One segment of a media playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Twitch's sequence number for the segment.
    pub n: u64,
    /// Seconds.
    pub dur: f64,
    /// The `#EXTINF` title (`live`, `Amazon|id`...).
    pub title: String,
    /// The `#EXT-X-PROGRAM-DATE-TIME` text, when present.
    pub pdt: Option<String>,
    /// The same as seconds since the epoch.
    pub when: Option<f64>,
    /// The segment's address.
    pub url: String,
    /// `true` for an advert segment.
    pub ad: bool,
    /// `true` when a `#EXT-X-DISCONTINUITY` preceded it.
    pub disc: bool,
    /// The init piece (`#EXT-X-MAP`) for fMP4 channels; a player cannot decode without it.
    pub map: Option<String>,
}

/// The advert break a media playlist announces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdBreak {
    /// `preroll`, `midroll`, or `advert` when Twitch did not say.
    pub roll: String,
    /// `X-TV-TWITCH-AD-POD-LENGTH`, or empty.
    pub pod: String,
    /// The announced seconds, as Twitch wrote them, or empty.
    pub seconds: String,
}

/// A media playlist as data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Media {
    /// `#EXT-X-MEDIA-SEQUENCE`.
    pub seq: u64,
    /// `#EXT-X-TARGETDURATION`, 6 when missing.
    pub target: u64,
    /// The segments in order.
    pub segments: Vec<Segment>,
    /// The advert break, if any segment is an advert or a window was announced.
    pub ads: Option<AdBreak>,
    /// Segment titles that are neither `live` nor a known advert shape, sorted.
    pub odd_titles: Vec<String>,
    /// `DATERANGE` classes in neither list, sorted and cut to 60 characters.
    pub odd_classes: Vec<String>,
    /// `true` while Twitch asks the player to draw an advert of its own. Nothing is cut for it.
    pub client_side: bool,
}

/// The facts Twitch states about the viewer in a master playlist.
pub fn master_info(master: &str) -> MasterInfo {
    let line = master
        .lines()
        .find(|l| l.starts_with("#EXT-X-TWITCH-INFO:"))
        .unwrap_or("");
    let get = |key: &str| attr(line, key).unwrap_or("").to_string();
    MasterInfo {
        country: get("USER-COUNTRY"),
        cluster: get("CLUSTER"),
        node: get("NODE"),
        session: get("VIDEO-SESSION-ID"),
    }
}

/// The qualities a master playlist offers, best first.
pub fn variants(master: &str) -> Vec<Variant> {
    let mut names: Vec<(String, String)> = Vec::new();
    let mut out: Vec<Variant> = Vec::new();
    let mut info: Option<&str> = None;
    for line in master.lines() {
        if line.starts_with("#EXT-X-MEDIA:") {
            if let (Some(g), Some(n)) = (attr(line, "GROUP-ID"), attr(line, "NAME")) {
                names.push((g.to_string(), n.to_string()));
            }
        } else if line.starts_with("#EXT-X-STREAM-INF:") {
            info = Some(line);
        } else if !line.is_empty()
            && !line.starts_with('#')
            && let Some(inf) = info.take()
        {
            let get = |key: &str| attr(inf, key).unwrap_or("").to_string();
            let slug = match attr(inf, "VIDEO") {
                Some(v) if !v.is_empty() => v.to_string(),
                _ => format!("v{}", out.len()),
            };
            let name = names
                .iter()
                .find(|(g, _)| *g == slug)
                .map_or_else(|| slug.clone(), |(_, n)| n.clone());
            out.push(Variant {
                bandwidth: get("BANDWIDTH").parse().unwrap_or(0),
                resolution: get("RESOLUTION"),
                fps: get("FRAME-RATE"),
                codecs: get("CODECS"),
                url: line.to_string(),
                slug,
                name,
            });
        }
    }
    out.sort_by_key(|v| std::cmp::Reverse(v.bandwidth));
    out
}

fn is_ad_daterange(line: &str) -> bool {
    line.contains(r#"CLASS="twitch-stitched"#) || line.contains(r#"ID="stitched-ad-"#)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.is_empty())
}

/// Reads one advert `DATERANGE`: its window, and the break it announces if none is known yet.
fn advert_window(line: &str, windows: &mut Vec<(f64, f64)>, ads: &mut Option<AdBreak>) {
    let duration = non_empty(attr(line, "DURATION"));
    let filled = non_empty(attr(line, "X-TV-TWITCH-AD-POD-FILLED-DURATION"));
    let length: f64 = duration
        .or(filled)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    if let Some(start) = attr(line, "START-DATE").and_then(when)
        && length != 0.0
    {
        windows.push((start, start + length));
    }
    if ads.is_none() {
        let roll = attr(line, "X-TV-TWITCH-AD-ROLL-TYPE")
            .unwrap_or("")
            .to_lowercase();
        *ads = Some(AdBreak {
            roll: if roll.is_empty() {
                "advert".to_string()
            } else {
                roll
            },
            pod: attr(line, "X-TV-TWITCH-AD-POD-LENGTH")
                .unwrap_or("")
                .to_string(),
            seconds: filled.or(duration).unwrap_or("").to_string(),
        });
    }
}

fn truncate(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// A media playlist as data. See the module documentation for what counts as an advert.
pub fn parse_media(text: &str) -> Media {
    let mut media = Media {
        target: 6,
        client_side: text.contains(CLIENT_SIDE),
        ..Media::default()
    };
    let mut windows: Vec<(f64, f64)> = Vec::new();
    let mut odd_titles = BTreeSet::new();
    let mut odd_classes = BTreeSet::new();
    let mut cue_out = false;
    let mut pending: Option<(f64, String)> = None;
    let mut pdt: Option<String> = None;
    let mut disc = false;
    let mut init: Option<String> = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            media.seq = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            media.target = v.trim().parse::<f64>().map_or(6, |f| f as u64);
        } else if line.starts_with("#EXT-X-DATERANGE:") {
            if is_ad_daterange(line) {
                advert_window(line, &mut windows, &mut media.ads);
            } else if let Some(class) = attr(line, "CLASS")
                && !KNOWN_CLASSES.contains(&class)
            {
                odd_classes.insert(truncate(class, 60));
            }
        } else if line.starts_with("#EXT-X-CUE-OUT") {
            cue_out = true;
        } else if line.starts_with("#EXT-X-CUE-IN") {
            cue_out = false;
        } else if line.starts_with("#EXT-X-DISCONTINUITY")
            && !line.starts_with("#EXT-X-DISCONTINUITY-SEQUENCE")
        {
            disc = true;
        } else if line.starts_with("#EXT-X-MAP:") {
            init = non_empty(attr(line, "URI")).map(str::to_string);
        } else if let Some(v) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            pdt = non_empty(Some(v)).map(str::to_string);
        } else if let Some(v) = line.strip_prefix("#EXTINF:") {
            let (d, title) = v.split_once(',').unwrap_or((v, ""));
            pending = Some((d.trim().parse().unwrap_or(0.0), title.to_string()));
        } else if !line.is_empty() && !line.starts_with('#') {
            let Some((dur, title)) = pending.take() else {
                continue;
            };
            let at = pdt.as_deref().and_then(when);
            let known_ad = title.contains('|')
                || title.starts_with("Amazon")
                || title.starts_with("DCM")
                || cue_out
                || AD_URLS.iter().any(|p| line.contains(p));
            let in_window =
                at.is_some_and(|t| windows.iter().any(|(a, b)| a - 0.05 <= t && t < b - 0.05));
            if !known_ad && !STREAM_TITLES.contains(&title.as_str()) {
                odd_titles.insert(title.clone());
            }
            media.segments.push(Segment {
                n: media.seq + media.segments.len() as u64,
                dur,
                title,
                pdt: pdt.take(),
                when: at,
                url: line.to_string(),
                ad: known_ad || in_window,
                disc,
                map: init.clone(),
            });
            disc = false;
        }
    }
    if media.ads.is_none() && media.segments.iter().any(|s| s.ad) {
        media.ads = Some(AdBreak {
            roll: "advert".to_string(),
            pod: String::new(),
            seconds: String::new(),
        });
    }
    media.odd_titles = odd_titles.into_iter().collect();
    media.odd_classes = odd_classes.into_iter().collect();
    media
}

/// `true` when a media playlist announces or contains an advert.
pub fn has_ads(media_text: &str) -> bool {
    let m = parse_media(media_text);
    m.ads.is_some() || m.segments.iter().any(|s| s.ad)
}
