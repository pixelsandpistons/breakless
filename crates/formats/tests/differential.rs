#![allow(clippy::assert_is_empty)] // `assert!(x.is_empty())` reads as the sentence it proves

//! The Rust parser against what the Python service produced for the same fixtures.
//!
//! `fixtures/expected/` was written once by `fixtures/gen-expected.py` from the Python
//! implementation this crate replaces. For every media playlist: the rebuilt playlist, byte for
//! byte, and the facts the service reads. For every master playlist: the qualities and the country.

mod common;

use std::collections::BTreeMap;

use breakless_formats::{Timeline, master_info, parse_media, variants};
use common::{fixture, fixtures};

fn names(suffix: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(fixtures().join("playlists"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(suffix))
        .map(|n| n.trim_end_matches(".m3u8").to_string())
        .collect();
    out.sort();
    out
}

fn facts(name: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in fixture(&format!("expected/{name}.facts")).lines() {
        if let Some((k, v)) = line.split_once('=') {
            out.entry(k.to_string()).or_default().push(v.to_string());
        }
    }
    out
}

fn one<'a>(f: &'a BTreeMap<String, Vec<String>>, key: &str) -> &'a str {
    f.get(key)
        .and_then(|v| v.first())
        .map_or("", String::as_str)
}

#[test]
fn media_playlists_render_as_the_python_service_did() {
    let list = names("-media.m3u8");
    assert!(!list.is_empty(), "no fixtures under fixtures/playlists");
    for name in list {
        let m = parse_media(&fixture(&format!("playlists/{name}.m3u8")));
        let mut line = Timeline::default();
        let (added, cut) = line.feed("s1", &m);
        assert_eq!(
            line.render(),
            fixture(&format!("expected/{name}.render.m3u8")),
            "{name}: render"
        );
        let f = facts(&name);
        assert_eq!(m.seq.to_string(), one(&f, "seq"), "{name}: seq");
        assert_eq!(m.target.to_string(), one(&f, "target"), "{name}: target");
        assert_eq!(
            m.segments.len().to_string(),
            one(&f, "segments"),
            "{name}: segments"
        );
        let ads = m.ads.as_ref().map_or("none".to_string(), |a| {
            format!("{}|{}|{}", a.roll, a.pod, a.seconds)
        });
        assert_eq!(ads, one(&f, "ads"), "{name}: ads");
        assert_eq!(added.to_string(), one(&f, "added"), "{name}: added");
        assert_eq!(cut.to_string(), one(&f, "cut"), "{name}: cut");
        assert_eq!(
            m.client_side.to_string(),
            one(&f, "client_side"),
            "{name}: client_side"
        );
        assert_eq!(
            m.odd_titles.join(","),
            one(&f, "odd_titles"),
            "{name}: odd_titles"
        );
        assert_eq!(
            m.odd_classes.join(","),
            one(&f, "odd_classes"),
            "{name}: odd_classes"
        );
    }
}

#[test]
fn master_playlists_read_as_the_python_service_did() {
    let list = names("-master.m3u8");
    assert!(!list.is_empty(), "no fixtures under fixtures/playlists");
    for name in list {
        let text = fixture(&format!("playlists/{name}.m3u8"));
        let f = facts(&name);
        let info = master_info(&text);
        assert_eq!(info.country, one(&f, "country"), "{name}");
        assert_eq!(info.cluster, one(&f, "cluster"), "{name}");
        let got: Vec<String> = variants(&text)
            .iter()
            .map(|v| {
                format!(
                    "{}|{}|{}|{}|{}|{}",
                    v.slug, v.name, v.bandwidth, v.resolution, v.fps, v.codecs
                )
            })
            .collect();
        assert_eq!(
            got,
            f.get("variant").cloned().unwrap_or_default(),
            "{name}: variants"
        );
    }
}
