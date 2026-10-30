#![allow(dead_code)] // each test binary uses a different subset

//! Shared by the integration tests: the fixtures folder and a made-up media playlist.

use std::path::PathBuf;

/// The repository's `fixtures/` folder.
pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// A fixture's text; panics with its name when it is missing.
pub fn fixture(relative: &str) -> String {
    let path = fixtures().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A media playlist whose segments are `(seconds past 12:00:00, title)`.
///
/// A title other than `live` is an advert segment; a change between advert and stream gets a
/// `#EXT-X-DISCONTINUITY`, as Twitch writes one. `extra` is inserted after the header.
pub fn playlist(first: u64, segs: &[(u32, &str)], extra: &str) -> String {
    let mut out = vec![
        "#EXTM3U".to_string(),
        "#EXT-X-VERSION:3".to_string(),
        "#EXT-X-TARGETDURATION:6".to_string(),
        format!("#EXT-X-MEDIA-SEQUENCE:{first}"),
        extra.to_string(),
    ];
    let mut was_ad: Option<bool> = None;
    for (sec, title) in segs {
        let ad = *title != "live";
        if was_ad.is_some_and(|w| w != ad) {
            out.push("#EXT-X-DISCONTINUITY".to_string());
        }
        was_ad = Some(ad);
        out.push(format!(
            "#EXT-X-PROGRAM-DATE-TIME:2026-10-04T12:{:02}:{:02}.000Z",
            sec / 60,
            sec % 60
        ));
        out.push(format!("#EXTINF:2.000,{title}"));
        out.push(format!(
            "https://cdn.example/{}-{sec}.ts",
            if ad { "ad" } else { "live" }
        ));
    }
    let mut text = out
        .into_iter()
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    text.push('\n');
    text
}
