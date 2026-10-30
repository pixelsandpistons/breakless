#![allow(clippy::assert_is_empty)] // `assert!(x.is_empty())` reads as the sentence it proves

//! Parsing: the real playlists under `fixtures/playlists/` and made-up advert breaks.

mod common;

use breakless_formats::{Timeline, master_info, parse_media, variants};
use common::{fixture, fixtures, playlist};

fn media_fixtures() -> Vec<(String, String)> {
    let dir = fixtures().join("playlists");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with("-media.m3u8"))
        .map(|n| {
            let text = std::fs::read_to_string(dir.join(&n)).unwrap();
            (n, text)
        })
        .collect();
    out.sort();
    out
}

fn ads(text: &str) -> Vec<bool> {
    parse_media(text).segments.iter().map(|s| s.ad).collect()
}

// -- the samples ------------------------------------------------------------------------------

#[test]
fn every_sample_parses_with_no_unknown_title_or_class() {
    let files = media_fixtures();
    assert!(!files.is_empty(), "no fixtures under fixtures/playlists");
    for (name, text) in files {
        let m = parse_media(&text);
        assert!(!m.segments.is_empty(), "{name}");
        assert!(m.odd_titles.is_empty(), "{name}: {:?}", m.odd_titles);
        assert!(m.odd_classes.is_empty(), "{name}: {:?}", m.odd_classes);
    }
}

#[test]
fn the_clean_sample_has_no_adverts() {
    let m = parse_media(&fixture("playlists/home-clean-media.m3u8"));
    assert!(m.ads.is_none());
    assert!(m.segments.iter().all(|s| !s.ad && s.title == "live"));
}

#[test]
fn the_preroll_sample_is_all_adverts_and_renders_nothing() {
    let m = parse_media(&fixture("playlists/home-preroll-media.m3u8"));
    assert!(m.ads.is_some());
    assert!(m.segments.iter().all(|s| s.ad));
    let mut line = Timeline::default();
    assert_eq!(line.feed("s1", &m), (0, m.segments.len() as u64));
    assert!(!line.render().contains("https://"));
}

#[test]
fn a_client_side_advert_request_cuts_nothing() {
    let m = parse_media(&fixture("playlists/home-clientside-media.m3u8"));
    assert!(m.client_side);
    assert!(m.ads.is_none());
    assert!(m.segments.iter().all(|s| !s.ad));
    let mut line = Timeline::default();
    assert_eq!(line.feed("s1", &m), (m.segments.len() as u64, 0));
}

#[test]
fn the_master_sample_lists_qualities_best_first_and_states_the_country() {
    let text = fixture("playlists/home-clean-master.m3u8");
    let v = variants(&text);
    assert_eq!(v[0].slug, "chunked");
    assert_eq!(v[v.len() - 1].slug, "audio_only");
    let bandwidths: Vec<u64> = v.iter().map(|x| x.bandwidth).collect();
    let mut sorted = bandwidths.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(bandwidths, sorted);
    assert_eq!(master_info(&text).country, "CA");
}

// -- advert marks -----------------------------------------------------------------------------

#[test]
fn advert_titles_mark_segments() {
    let text = playlist(
        10,
        &[(0, "live"), (2, "Amazon|123"), (4, "DCM|9"), (6, "live")],
        "",
    );
    assert_eq!(ads(&text), [false, true, true, false]);
}

#[test]
fn an_advert_window_marks_segments_whatever_their_title() {
    let tag = r#"#EXT-X-DATERANGE:ID="stitched-ad-1-2",CLASS="twitch-stitched-ad",START-DATE="2026-10-04T12:00:02.000Z",DURATION=4.000,X-TV-TWITCH-AD-ROLL-TYPE="MIDROLL""#;
    let text = playlist(
        10,
        &[(0, "live"), (2, "live"), (4, "live"), (6, "live")],
        tag,
    );
    let m = parse_media(&text);
    assert_eq!(
        m.segments.iter().map(|s| s.ad).collect::<Vec<_>>(),
        [false, true, true, false]
    );
    assert_eq!(m.ads.unwrap().roll, "midroll");
}

#[test]
fn an_unknown_title_is_reported_not_cut() {
    let m = parse_media(&playlist(10, &[(0, "live"), (2, "somethingnew")], ""));
    assert_eq!(m.odd_titles, ["somethingnew"]);
    assert!(!m.segments[1].ad);
}

// -- marks other tools look for that were never seen here (from vaft 68.5.7) -------------------

#[test]
fn the_whole_stitched_family_is_an_advert_window() {
    let tag = r#"#EXT-X-DATERANGE:ID="x-1",CLASS="twitch-stitched-mid",START-DATE="2026-10-04T12:00:02.000Z",DURATION=4.000"#;
    let m = parse_media(&playlist(
        10,
        &[(0, "live"), (2, "live"), (4, "live"), (6, "live")],
        tag,
    ));
    assert_eq!(
        m.segments.iter().map(|s| s.ad).collect::<Vec<_>>(),
        [false, true, true, false]
    );
    assert!(m.odd_classes.is_empty());
}

#[test]
fn a_segment_address_that_says_advert() {
    let text = playlist(10, &[(0, "live"), (2, "live")], "").replace(
        "https://cdn.example/live-2.ts",
        "https://cdn.example/adsquared/2.ts",
    );
    let m = parse_media(&text);
    assert_eq!(
        m.segments.iter().map(|s| s.ad).collect::<Vec<_>>(),
        [false, true]
    );
    assert!(m.ads.is_some());
}

#[test]
fn cue_out_to_cue_in() {
    let text = playlist(10, &[(0, "live"), (2, "live"), (4, "live")], "");
    let mut lines: Vec<&str> = text.lines().collect();
    let i = lines
        .iter()
        .position(|l| *l == "https://cdn.example/live-0.ts")
        .unwrap();
    lines.insert(i + 1, "#EXT-X-CUE-OUT:DURATION=2");
    let j = lines
        .iter()
        .position(|l| *l == "https://cdn.example/live-2.ts")
        .unwrap();
    lines.insert(j + 1, "#EXT-X-CUE-IN");
    assert_eq!(ads(&(lines.join("\n") + "\n")), [false, true, false]);
}

#[test]
fn a_class_never_met_is_reported_and_cuts_nothing() {
    let tag = r#"#EXT-X-DATERANGE:ID="n-1",CLASS="twitch-something-new",START-DATE="2026-10-04T12:00:02.000Z",DURATION=4.000"#;
    let m = parse_media(&playlist(10, &[(0, "live"), (2, "live")], tag));
    assert_eq!(m.odd_classes, ["twitch-something-new"]);
    assert_eq!(
        m.segments.iter().map(|s| s.ad).collect::<Vec<_>>(),
        [false, false]
    );
}

// -- fMP4 channels ----------------------------------------------------------------------------

const FMP4: &str = "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:100\n\
#EXT-X-MAP:URI=\"https://a.hls.ttvnw.net/v1/segment/INIT1.mp4\"\n\
#EXT-X-PROGRAM-DATE-TIME:2026-10-08T14:00:00.000Z\n#EXTINF:2.000,live\nhttps://a.hls.ttvnw.net/v1/segment/S100.mp4\n\
#EXT-X-PROGRAM-DATE-TIME:2026-10-08T14:00:02.000Z\n#EXTINF:2.000,live\nhttps://a.hls.ttvnw.net/v1/segment/S101.mp4\n\
#EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI=\"https://a.hls.ttvnw.net/v1/segment/INIT2.mp4\"\n\
#EXT-X-PROGRAM-DATE-TIME:2026-10-08T14:00:04.000Z\n#EXTINF:2.000,live\nhttps://a.hls.ttvnw.net/v1/segment/S102.mp4\n";

#[test]
fn each_segment_knows_its_init_piece() {
    let m = parse_media(FMP4);
    let maps: Vec<&str> = m
        .segments
        .iter()
        .map(|s| s.map.as_deref().unwrap().rsplit('/').next().unwrap())
        .collect();
    assert_eq!(maps, ["INIT1.mp4", "INIT1.mp4", "INIT2.mp4"]);
}

#[test]
fn the_playlist_handed_on_names_the_init_piece_before_the_segments_it_serves() {
    let mut line = Timeline::default();
    line.feed("a", &parse_media(FMP4));
    let text = line.render();
    let out: Vec<&str> = text.lines().collect();
    assert!(out.contains(&"#EXT-X-VERSION:6"));
    let maps: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("#EXT-X-MAP:"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(maps.len(), 2);
    let at = |url: &str| out.iter().position(|l| *l == url).unwrap();
    assert!(maps[0] < at("https://a.hls.ttvnw.net/v1/segment/S100.mp4"));
    assert!(at("https://a.hls.ttvnw.net/v1/segment/S101.mp4") < maps[1]);
    assert!(maps[1] < at("https://a.hls.ttvnw.net/v1/segment/S102.mp4"));
}

#[test]
fn an_old_style_channel_is_written_as_before() {
    let mut line = Timeline::default();
    line.feed(
        "a",
        &parse_media(&playlist(10, &[(0, "live"), (2, "live")], "")),
    );
    let text = line.render();
    assert!(text.contains("#EXT-X-VERSION:3"));
    assert!(!text.contains("#EXT-X-MAP"));
}
