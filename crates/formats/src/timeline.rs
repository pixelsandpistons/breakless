//! The stream as we hand it on: stream segments only, numbered by us.
//!
//! Twitch numbers adverts and stream alike, so cutting adverts out of its playlist leaves holes a
//! player trips over. [`Timeline::feed`] takes each fresh playlist and appends only the stream
//! segments not seen before, giving them our own running number; [`Timeline::render`] writes the
//! last `keep` of them. While a playlist is all adverts nothing is appended and `render` keeps
//! returning the same playlist: the player waits on the last picture instead of failing. A break
//! where adverts were cut, or where the session changed, is marked with a discontinuity.

use std::collections::{HashSet, VecDeque};

use crate::playlist::Media;

const SEEN_LIMIT: usize = 600;

#[derive(Debug, Clone)]
struct Out {
    n: u64,
    dur: f64,
    pdt: Option<String>,
    url: String,
    disc: bool,
    map: Option<String>,
}

/// One quality's rebuilt playlist. See the module documentation.
#[derive(Debug, Clone)]
pub struct Timeline {
    segs: VecDeque<Out>,
    keep: usize,
    next_n: u64,
    disc_base: u64,
    target: u64,
    last_when: Option<f64>,
    last_session: Option<String>,
    gap: bool,
    seen: HashSet<(String, u64)>,
    seen_order: VecDeque<(String, u64)>,
    /// Advert segments dropped, ever.
    pub cut: u64,
    /// Stream segments passed on, ever.
    pub passed: u64,
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new(15)
    }
}

impl Timeline {
    /// A timeline that renders the last `keep` segments.
    pub fn new(keep: usize) -> Self {
        Self {
            segs: VecDeque::new(),
            keep,
            next_n: 0,
            disc_base: 0,
            target: 6,
            last_when: None,
            last_session: None,
            gap: false,
            seen: HashSet::new(),
            seen_order: VecDeque::new(),
            cut: 0,
            passed: 0,
        }
    }

    /// The segments currently in view.
    pub fn len(&self) -> usize {
        self.segs.len()
    }

    /// `true` before any stream segment arrived.
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// Feeds one fresh playlist from `session`. Returns `(stream segments added, advert segments cut)`.
    pub fn feed(&mut self, session: &str, media: &Media) -> (u64, u64) {
        let (mut added, mut cut) = (0, 0);
        self.target = self.target.max(media.target);
        for s in &media.segments {
            let key = (session.to_string(), s.n);
            if self.seen.contains(&key) {
                continue;
            }
            self.seen.insert(key.clone());
            self.seen_order.push_back(key);
            if self.seen_order.len() > SEEN_LIMIT
                && let Some(old) = self.seen_order.pop_front()
            {
                self.seen.remove(&old);
            }
            if s.ad {
                cut += 1;
                self.gap = true;
                continue;
            }
            if let (Some(last), Some(at)) = (self.last_when, s.when)
                && at <= last + 0.05
            {
                continue; // a new session's playlist overlaps what was already handed on
            }
            let changed = self.last_session.as_deref().is_some_and(|l| l != session);
            let disc = !self.segs.is_empty() && (self.gap || changed || s.disc);
            self.segs.push_back(Out {
                n: self.next_n,
                dur: s.dur,
                pdt: s.pdt.clone(),
                url: s.url.clone(),
                disc,
                map: s.map.clone(),
            });
            self.next_n += 1;
            self.gap = false;
            self.last_session = Some(session.to_string());
            if s.when.is_some() {
                self.last_when = s.when;
            }
            added += 1;
            while self.segs.len() > self.keep {
                if let Some(gone) = self.segs.pop_front()
                    && gone.disc
                {
                    self.disc_base += 1;
                }
            }
        }
        self.cut += cut;
        self.passed += added;
        (added, cut)
    }

    /// The playlist to hand a player.
    ///
    /// fMP4 channels: each segment needs its init piece named first (`#EXT-X-MAP`, which needs
    /// version 6). It is written before the first segment and again whenever it changes (a new
    /// session, or the stream after a cut).
    pub fn render(&self) -> String {
        let fmp4 = self.segs.iter().any(|s| s.map.is_some());
        let first = self.segs.front().map_or(self.next_n, |s| s.n);
        let mut out: Vec<String> = vec![
            "#EXTM3U".to_string(),
            format!("#EXT-X-VERSION:{}", if fmp4 { 6 } else { 3 }),
            format!("#EXT-X-TARGETDURATION:{}", self.target),
            format!("#EXT-X-MEDIA-SEQUENCE:{first}"),
            format!("#EXT-X-DISCONTINUITY-SEQUENCE:{}", self.disc_base),
        ];
        let mut init: Option<&str> = None;
        for (i, s) in self.segs.iter().enumerate() {
            if s.disc && i > 0 {
                out.push("#EXT-X-DISCONTINUITY".to_string());
            } else if s.disc {
                // the first segment in view starts a new run: already counted in disc_base + 1
                out[4] = format!("#EXT-X-DISCONTINUITY-SEQUENCE:{}", self.disc_base + 1);
            }
            if let Some(map) = s.map.as_deref()
                && init != Some(map)
            {
                init = Some(map);
                out.push(format!("#EXT-X-MAP:URI=\"{map}\""));
            }
            if let Some(pdt) = &s.pdt {
                out.push(format!("#EXT-X-PROGRAM-DATE-TIME:{pdt}"));
            }
            out.push(format!("#EXTINF:{:.3},live", s.dur));
            out.push(s.url.clone());
        }
        let mut text = out.join("\n");
        text.push('\n');
        text
    }
}
