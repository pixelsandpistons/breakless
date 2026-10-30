//! The rebuilt playlist: our numbering, cuts, held playlists, session changes.

mod common;

use breakless_formats::{Timeline, parse_media};
use common::playlist;

fn urls(line: &Timeline) -> Vec<String> {
    line.render()
        .lines()
        .filter(|l| l.starts_with("https://"))
        .map(str::to_string)
        .collect()
}

fn seq(line: &Timeline) -> u64 {
    line.render()
        .lines()
        .find(|l| l.starts_with("#EXT-X-MEDIA-SEQUENCE"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap()
}

#[test]
fn a_midroll_is_cut_and_numbering_never_goes_back() {
    let mut line = Timeline::new(8);
    // the window slides two seconds at a time through: stream, a 6 s advert, stream again
    let mut story: Vec<(u32, &str)> = (0..12).step_by(2).map(|s| (s, "live")).collect();
    story.extend((12..18).step_by(2).map(|s| (s, "Amazon|1")));
    story.extend((18..30).step_by(2).map(|s| (s, "live")));
    let mut last_seq = 0;
    for start in 0..story.len() - 4 {
        line.feed(
            "s1",
            &parse_media(&playlist(100 + start as u64, &story[start..start + 5], "")),
        );
        assert!(urls(&line).iter().all(|u| !u.contains("ad-")));
        assert!(seq(&line) >= last_seq);
        last_seq = seq(&line);
    }
    assert_eq!(line.cut, 3);
    assert_eq!(line.passed, 12);
    let text = line.render();
    assert_eq!(text.matches("#EXT-X-DISCONTINUITY\n").count(), 1);
    let disc = text.find("#EXT-X-DISCONTINUITY\n").unwrap();
    assert!(text.find("live-10.ts").unwrap() < disc);
    assert!(disc < text.find("live-18.ts").unwrap());
}

#[test]
fn the_playlist_holds_still_while_only_adverts_arrive() {
    let mut line = Timeline::default();
    line.feed(
        "s1",
        &parse_media(&playlist(1, &[(0, "live"), (2, "live")], "")),
    );
    let held = line.render();
    for i in 0..3u32 {
        let segs = [(4 + 2 * i, "DCM|7"), (6 + 2 * i, "DCM|7")];
        line.feed("s1", &parse_media(&playlist(3 + u64::from(i), &segs, "")));
        assert_eq!(line.render(), held);
    }
}

#[test]
fn the_same_playlist_twice_adds_nothing() {
    let mut line = Timeline::default();
    let p = parse_media(&playlist(1, &[(0, "live"), (2, "live")], ""));
    assert_eq!(line.feed("s1", &p), (2, 0));
    assert_eq!(line.feed("s1", &p), (0, 0));
}

#[test]
fn a_new_session_overlapping_in_time_is_not_repeated() {
    let mut line = Timeline::default();
    line.feed(
        "s1",
        &parse_media(&playlist(50, &[(0, "live"), (2, "live"), (4, "live")], "")),
    );
    // a new session numbers from somewhere else and its window overlaps the old one
    line.feed(
        "s2",
        &parse_media(&playlist(
            900,
            &[(2, "live"), (4, "live"), (6, "live"), (8, "live")],
            "",
        )),
    );
    let tails: Vec<String> = urls(&line)
        .iter()
        .map(|u| u.rsplit('-').next().unwrap().to_string())
        .collect();
    assert_eq!(tails, ["0.ts", "2.ts", "4.ts", "6.ts", "8.ts"]);
    let text = line.render();
    assert_eq!(text.matches("#EXT-X-DISCONTINUITY\n").count(), 1);
    assert!(text.find("#EXT-X-DISCONTINUITY\n").unwrap() < text.find("live-6.ts").unwrap());
}

#[test]
fn the_discontinuity_sequence_counts_what_slid_out() {
    let mut line = Timeline::new(2);
    line.feed(
        "s1",
        &parse_media(&playlist(
            1,
            &[
                (0, "live"),
                (2, "Amazon|1"),
                (4, "live"),
                (6, "live"),
                (8, "live"),
            ],
            "",
        )),
    );
    let text = line.render();
    // live-4 began a new run and has slid out of view: one discontinuity is behind us
    assert!(text.contains("#EXT-X-DISCONTINUITY-SEQUENCE:1"));
    assert!(!text.contains("#EXT-X-DISCONTINUITY\n"));
    assert_eq!(seq(&line), 2);
}
