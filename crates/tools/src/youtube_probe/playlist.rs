//! Measurements and scrubbed captures of HLS, without deciding which marks mean adverts.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};

use breakless_formats::attrs::attr;
use reqwest::Url;

/// Distinct segments, discontinuities, classes and hosts observed across a sliding window.
#[derive(Debug, Default)]
pub(super) struct Counts {
    /// Segments observed once each, using media sequence numbers where present.
    pub segments: u64,
    /// Discontinuity boundaries observed once each.
    pub discontinuities: u64,
    /// Every class, with an explicit marker when a DATERANGE has no class.
    pub classes: BTreeSet<String>,
    /// Segment hosts after resolving relative references against the media URL.
    pub hosts: BTreeSet<String>,
    seen_segments: BTreeSet<String>,
    seen_discontinuities: BTreeSet<String>,
}

impl Counts {
    /// Adds one media playlist; polling the same window again adds nothing to the counters.
    pub fn feed(&mut self, text: &str, base: &Url) {
        let sequence = number(text, "#EXT-X-MEDIA-SEQUENCE:");
        let mut discontinuity = number(text, "#EXT-X-DISCONTINUITY-SEQUENCE:");
        let mut segment = 0_u64;
        let mut pending = false;
        let mut boundaries = 0_u64;
        for line in text.lines().map(str::trim) {
            if line == "#EXT-X-DISCONTINUITY" {
                if let Some(n) = &mut discontinuity {
                    self.record_discontinuity(format!("discontinuity:{n}"));
                    *n = n.saturating_add(1);
                } else {
                    boundaries += 1;
                }
            } else if line.starts_with("#EXT-X-DATERANGE:") {
                let class = attr(line, "CLASS").unwrap_or("(missing)");
                self.classes
                    .insert(if class.is_empty() { "(empty)" } else { class }.to_string());
            } else if line.starts_with("#EXTINF:") {
                pending = true;
            } else if pending && !line.is_empty() && !line.starts_with('#') {
                let url = base.join(line).ok();
                let key = sequence.map_or_else(
                    || {
                        url.as_ref()
                            .map_or_else(|| line.to_string(), ToString::to_string)
                    },
                    |n| format!("sequence:{}", n.saturating_add(segment)),
                );
                if self.seen_segments.insert(key.clone()) {
                    self.segments += 1;
                }
                for n in 0..boundaries {
                    self.record_discontinuity(format!("segment:{key}:{n}"));
                }
                boundaries = 0;
                self.hosts.insert(
                    url.as_ref()
                        .and_then(Url::host_str)
                        .unwrap_or("(invalid)")
                        .to_string(),
                );
                segment += 1;
                pending = false;
            }
        }
        // A boundary can precede a segment that has not arrived yet.
        if let Some(n) = sequence {
            for ordinal in 0..boundaries {
                self.record_discontinuity(format!(
                    "segment:sequence:{}:{ordinal}",
                    n.saturating_add(segment)
                ));
            }
        }
    }

    fn record_discontinuity(&mut self, key: String) {
        if self.seen_discontinuities.insert(key) {
            self.discontinuities += 1;
        }
    }
}

fn number(text: &str, prefix: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix(prefix)?.parse().ok())
}

fn sensitive(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "sig" | "signature" | "ip" | "expire" | "id"
    )
}

fn token_like(value: &str) -> bool {
    value.len() > 40
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'=' | b'.' | b'+' | b'/')
        })
}

fn address(value: &str) -> bool {
    let bare = value.trim_matches(['[', ']']);
    bare.parse::<IpAddr>().is_ok() || value.parse::<SocketAddr>().is_ok()
}

fn decode(value: &str) -> String {
    let mut bytes = Vec::new();
    let mut rest = value.as_bytes();
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'%' && rest.len() >= 3 {
            let high = char::from(rest[1]).to_digit(16);
            let low = char::from(rest[2]).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                bytes.push((high * 16 + low) as u8);
                rest = &rest[3..];
                continue;
            }
        }
        bytes.push(first);
        rest = tail;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn scrub_atom(value: &str) -> String {
    let decoded = decode(value);
    if address(&decoded) {
        "203.0.113.7".to_string()
    } else if token_like(&decoded) {
        "(removed)".to_string()
    } else {
        value.to_string()
    }
}

fn scrub_value(value: &str) -> String {
    let mut out = String::new();
    let mut atom = String::new();
    for c in value.chars() {
        if c.is_ascii_alphanumeric()
            || matches!(c, '_' | '-' | '=' | '.' | ':' | '%' | '[' | ']' | '+' | '/')
        {
            atom.push(c);
        } else {
            out.push_str(&scrub_atom(&atom));
            atom.clear();
            out.push(c);
        }
    }
    out.push_str(&scrub_atom(&atom));
    out
}

fn rename(value: &str, identities: &[&str]) -> String {
    let mut clean = value.to_string();
    for identity in identities.iter().filter(|identity| !identity.is_empty()) {
        clean = clean.replace(identity, "somechannel");
    }
    clean
}

fn scrub_reference(reference: &str, identities: &[&str]) -> String {
    let absolute = reference.starts_with("https://") || reference.starts_with("http://");
    let protocol_relative = reference.starts_with("//");
    let base = Url::parse("https://capture.invalid/").expect("constant capture base parses");
    let Ok(mut url) = base.join(reference) else {
        return scrub_value(reference);
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    if url.host_str().is_some_and(address) {
        let _ = url.set_host(Some("203.0.113.7"));
    }
    let mut redact_next = false;
    let segments = url
        .path_segments()
        .into_iter()
        .flatten()
        .map(|part| {
            let decoded = decode(part);
            let clean = if redact_next || decoded.len() > 30 {
                "(removed)".to_string()
            } else {
                scrub_value(&rename(&decoded, identities))
            };
            redact_next = sensitive(&decoded);
            clean
        })
        .collect::<Vec<_>>();
    if let Ok(mut path) = url.path_segments_mut() {
        path.clear().extend(&segments);
    }
    if url.query().is_some() {
        let pairs = url
            .query_pairs()
            .map(|(key, value)| {
                let clean = if sensitive(&key) {
                    "(removed)".to_string()
                } else {
                    scrub_value(&rename(&value, identities))
                };
                (key.into_owned(), clean)
            })
            .collect::<Vec<_>>();
        url.query_pairs_mut().clear().extend_pairs(pairs);
    }
    url.set_fragment(None);
    if absolute {
        url.to_string()
    } else if protocol_relative {
        url.as_str().trim_start_matches("https:").to_string()
    } else {
        let clean = url
            .as_str()
            .strip_prefix("https://capture.invalid/")
            .unwrap_or(url.as_str());
        if reference.starts_with('/') {
            format!("/{clean}")
        } else {
            clean.to_string()
        }
    }
}

fn scrub_tag(line: &str, identities: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(eq) = rest.find('=') {
        let start = rest[..eq].rfind([',', ':']).map_or(0, |i| i + 1);
        let key = &rest[start..eq];
        out.push_str(&rest[..=eq]);
        rest = &rest[eq + 1..];
        let quoted = rest.starts_with('"');
        if quoted {
            out.push('"');
            rest = &rest[1..];
        }
        let end = if quoted {
            rest.find('"')
        } else {
            rest.find(',')
        }
        .unwrap_or(rest.len());
        let value = &rest[..end];
        let clean = match key {
            "USER-IP" => "203.0.113.7".to_string(),
            "SERVING-ID" | "VIDEO-SESSION-ID" | "BROADCAST-ID" => "0".to_string(),
            "URI" | "URL" => scrub_reference(value, identities),
            key if sensitive(key) => "(removed)".to_string(),
            _ if value.starts_with("https://")
                || value.starts_with("http://")
                || value.starts_with("//") =>
            {
                scrub_reference(value, identities)
            }
            _ => scrub_value(value),
        };
        out.push_str(&clean);
        rest = &rest[end..];
        if quoted && rest.starts_with('"') {
            out.push('"');
            rest = &rest[1..];
        }
    }
    out.push_str(&scrub_value(rest));
    out
}

/// Scrubs viewer addresses, signatures, long token values and both channel and video identities.
pub(super) fn scrub(text: &str, identities: &[&str]) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut clean = if line.starts_with('#') {
            scrub_tag(line, identities)
        } else {
            scrub_reference(line, identities)
        };
        clean = rename(&clean, identities);
        out.push_str(&clean);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_windows_count_each_segment_and_discontinuity_once() {
        let base = Url::parse("https://cdn.example/live/media.m3u8").unwrap();
        let first = concat!(
            "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:8\n",
            "#EXT-X-DATERANGE:ID=\"one\",CLASS=\"unknown-mark\"\n",
            "#EXTINF:2.0,live\n8.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:2.0,live\n",
            "https://other.example/9.ts?sig=first\n"
        );
        let mut counts = Counts::default();
        counts.feed(first, &base);
        counts.feed(first, &base);
        counts.feed(
            concat!(
                "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:9\n",
                "#EXT-X-DISCONTINUITY\n#EXTINF:2.0,live\nhttps://other.example/9.ts?sig=second\n",
                "#EXTINF:2.0,live\n10.ts\n#EXT-X-DISCONTINUITY\n#EXTINF:2.0,live\n11.ts\n"
            ),
            &base,
        );
        assert_eq!(counts.segments, 4);
        assert_eq!(counts.discontinuities, 2);
        assert_eq!(counts.classes, BTreeSet::from(["unknown-mark".to_string()]));
        assert_eq!(
            counts.hosts,
            BTreeSet::from(["cdn.example".to_string(), "other.example".to_string()])
        );
    }

    #[test]
    fn explicit_discontinuity_sequence_survives_a_sliding_window() {
        let base = Url::parse("https://cdn.example/media.m3u8").unwrap();
        let mut counts = Counts::default();
        counts.feed("#EXT-X-MEDIA-SEQUENCE:1\n#EXT-X-DISCONTINUITY-SEQUENCE:10\n#EXT-X-DISCONTINUITY\n#EXTINF:2,\n1.ts\n", &base);
        counts.feed("#EXT-X-MEDIA-SEQUENCE:2\n#EXT-X-DISCONTINUITY-SEQUENCE:11\n#EXT-X-DISCONTINUITY\n#EXTINF:2,\n2.ts\n", &base);
        assert_eq!(counts.segments, 2);
        assert_eq!(counts.discontinuities, 2);
    }

    #[test]
    fn maps_are_not_segments_and_dateranges_without_classes_are_recorded() {
        let base = Url::parse("https://cdn.example/media.m3u8").unwrap();
        let mut counts = Counts::default();
        counts.feed(
            "#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-DATERANGE:ID=\"x\"\n#EXTINF:2,\nsegment.m4s\n",
            &base,
        );
        assert_eq!(counts.segments, 1);
        assert_eq!(counts.classes, BTreeSet::from(["(missing)".to_string()]));
    }

    #[test]
    fn master_capture_removes_query_signatures_addresses_and_identities() {
        let token = "A".repeat(41);
        let master = format!(
            "#EXTM3U\n#EXT-X-TWITCH-INFO:USER-IP=\"198.51.100.9\",SERVING-ID=\"session\",VIDEO-SESSION-ID=\"session\",BROADCAST-ID=\"session\",LONG=\"{token}\"\n#EXT-X-STREAM-INF:BANDWIDTH=1,NAME=\"ExampleChannel\"\nhttps://cdn.example/ExampleVideo/stream.m3u8?sig=secret&signature=secret&ip=198.51.100.9&expire=123&id=ExampleVideo&custom={token}&keep=1\n"
        );
        let clean = scrub(&master, &["ExampleVideo", "ExampleChannel"]);
        for private in [
            "secret",
            "198.51.100.9",
            "123",
            "ExampleVideo",
            "ExampleChannel",
            &token,
            "session",
        ] {
            assert!(!clean.contains(private), "{private}: {clean}");
        }
        assert!(clean.contains("USER-IP=\"203.0.113.7\""));
        assert!(clean.contains("SERVING-ID=\"0\""));
        assert!(clean.contains("NAME=\"somechannel\""));
        let url = Url::parse(clean.lines().last().unwrap()).unwrap();
        for (key, value) in url.query_pairs() {
            if key == "keep" {
                assert_eq!(value, "1");
            } else {
                assert_eq!(value, "(removed)");
            }
        }
    }

    #[test]
    fn media_capture_scrubs_slash_parameters_quoted_uris_and_encoded_tokens() {
        let token = "B".repeat(41);
        let media = format!(
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://keys.example/key?sig=key-secret&ip=2001%3Adb8%3A%3A7\"\n#EXT-X-MAP:URI=\"https://cdn.example/id/video-id/signature/map-secret/init.mp4\"\n#EXT-X-DATERANGE:ID=\"range\",CLASS=\"unmeasured-mark\",CUSTOM=\"{token}\",ADDRESS=\"2001:db8::7\"\n#EXTINF:2.0,live\nhttps://cdn.example/expire/123/ip/198.51.100.9/id/video-id/sig/media-secret/custom/{token}?other=%42{}\n",
            "B".repeat(40)
        );
        let clean = scrub(&media, &["video-id"]);
        for private in [
            "key-secret",
            "map-secret",
            "media-secret",
            "2001",
            "198.51.100.9",
            "123",
            "video-id",
            &token,
        ] {
            assert!(!clean.contains(private), "{private}: {clean}");
        }
        assert!(clean.contains("CLASS=\"unmeasured-mark\""));
        assert!(clean.contains(
            "https://cdn.example/expire/(removed)/ip/(removed)/id/(removed)/sig/(removed)"
        ));
        assert!(clean.contains("ADDRESS=\"203.0.113.7\""));
        assert!(!clean.contains("%42"));
    }
    #[test]
    fn capture_renames_encoded_identities_and_removes_base64_and_opaque_paths() {
        let base64 = format!("{}+/{}=", "C".repeat(20), "D".repeat(20));
        let path = "E".repeat(35);
        let text = format!(
            "#EXTM3U\n#EXT-X-MAP:URI=\"https://cdn.example/My%20Channel/init.mp4?channel=My+Channel\"\nhttps://cdn.example/My%20Channel/{path}/media.m3u8?channel=My%20Channel&blob={}\n",
            base64.replace('+', "%2B").replace('/', "%2F")
        );
        let clean = scrub(&text, &["My Channel"]);
        assert!(!clean.contains("My"), "{clean}");
        assert!(!clean.contains(&path), "{clean}");
        assert!(!clean.contains(&"C".repeat(20)), "{clean}");
        assert!(!clean.contains(&"D".repeat(20)), "{clean}");
        assert!(
            clean.contains("/somechannel/(removed)/media.m3u8"),
            "{clean}"
        );
        assert!(clean.contains("channel=somechannel"), "{clean}");
        assert_eq!(scrub_value(&base64), "(removed)");
    }
}
