#!/usr/bin/env python3
"""Tests for twitchlib.py that need no network: python3 twitch/test_twitchlib.py

They read the real playlists saved in fixtures/playlists/ (captured with capture.py) and
a made-up advert break in the middle of a stream. Add a sample when Twitch changes its format.
"""
import glob
import os
import unittest

import twitchlib as t

SAMPLES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "fixtures", "playlists")


def sample(name):
    with open(os.path.join(SAMPLES, name)) as f:
        return f.read()


def playlist(first, segs, extra=""):
    """A media playlist: segs is a list of (seconds past 12:00:00, title)."""
    out = ["#EXTM3U", "#EXT-X-VERSION:3", "#EXT-X-TARGETDURATION:6", "#EXT-X-MEDIA-SEQUENCE:%d" % first, extra]
    was_ad = None
    for i, (sec, title) in enumerate(segs):
        ad = title != "live"
        if was_ad is not None and ad != was_ad:
            out.append("#EXT-X-DISCONTINUITY")
        was_ad = ad
        out += ["#EXT-X-PROGRAM-DATE-TIME:2026-10-04T12:%02d:%02d.000Z" % (sec // 60, sec % 60),
                "#EXTINF:2.000," + title, "https://cdn.example/%s-%d.ts" % ("ad" if ad else "live", sec)]
    return "\n".join(x for x in out if x) + "\n"


class Samples(unittest.TestCase):
    def test_every_sample_parses(self):
        files = glob.glob(os.path.join(SAMPLES, "*-media.m3u8"))
        self.assertTrue(files)
        for f in files:
            m = t.parse_media(sample(os.path.basename(f)))
            self.assertTrue(m["segments"], f)
            self.assertEqual(m["odd_titles"], [], f)

    def test_clean_sample_has_no_adverts(self):
        m = t.parse_media(sample("home-clean-media.m3u8"))
        self.assertIsNone(m["ads"])
        self.assertFalse(any(s["ad"] for s in m["segments"]))
        self.assertTrue(all(s["title"] == "live" for s in m["segments"]))

    def test_preroll_sample_is_all_adverts(self):
        m = t.parse_media(sample("home-preroll-media.m3u8"))
        self.assertTrue(m["ads"])
        self.assertTrue(all(s["ad"] for s in m["segments"]))
        line = t.Timeline()
        self.assertEqual(line.feed("s1", m), (0, len(m["segments"])))
        self.assertNotIn("https://", line.render())

    def test_client_side_advert_request_cuts_nothing(self):
        m = t.parse_media(sample("home-clientside-media.m3u8"))
        self.assertTrue(m["client_side"])
        self.assertIsNone(m["ads"])
        self.assertFalse(any(s["ad"] for s in m["segments"]))
        line = t.Timeline()
        self.assertEqual(line.feed("s1", m), (len(m["segments"]), 0))

    def test_master_sample(self):
        text = sample("home-clean-master.m3u8")
        v = t.variants(text)
        self.assertEqual(v[0]["slug"], "chunked")
        self.assertEqual(v[-1]["slug"], "audio_only")
        self.assertEqual(sorted(x["bandwidth"] for x in v), [x["bandwidth"] for x in v][::-1])
        self.assertEqual(t.master_info(text)["country"], "CA")
        self.assertEqual(len(t.master_info(text)["country"]), 2)


class Adverts(unittest.TestCase):
    def test_titles(self):
        m = t.parse_media(playlist(10, [(0, "live"), (2, "Amazon|123"), (4, "DCM|9"), (6, "live")]))
        self.assertEqual([s["ad"] for s in m["segments"]], [False, True, True, False])

    def test_window_marks_segments_whatever_their_title(self):
        tag = ('#EXT-X-DATERANGE:ID="stitched-ad-1-2",CLASS="twitch-stitched-ad",'
               'START-DATE="2026-10-04T12:00:02.000Z",DURATION=4.000,X-TV-TWITCH-AD-ROLL-TYPE="MIDROLL"')
        m = t.parse_media(playlist(10, [(0, "live"), (2, "live"), (4, "live"), (6, "live")], tag))
        self.assertEqual([s["ad"] for s in m["segments"]], [False, True, True, False])
        self.assertEqual(m["ads"]["roll"], "midroll")

    def test_unknown_title_is_reported_not_cut(self):
        m = t.parse_media(playlist(10, [(0, "live"), (2, "somethingnew")]))
        self.assertEqual(m["odd_titles"], ["somethingnew"])
        self.assertFalse(m["segments"][1]["ad"])


class WiderMarks(unittest.TestCase):
    """Marks other tools look for that were never seen here (2026-10-10, from vaft 68.5.7)."""

    def test_the_whole_stitched_family_is_an_advert_window(self):
        tag = ('#EXT-X-DATERANGE:ID="x-1",CLASS="twitch-stitched-mid",'
               'START-DATE="2026-10-04T12:00:02.000Z",DURATION=4.000')
        m = t.parse_media(playlist(10, [(0, "live"), (2, "live"), (4, "live"), (6, "live")], tag))
        self.assertEqual([s["ad"] for s in m["segments"]], [False, True, True, False])
        self.assertEqual(m["odd_classes"], [])

    def test_a_segment_address_that_says_advert(self):
        text = playlist(10, [(0, "live"), (2, "live")]).replace("https://cdn.example/live-2.ts", "https://cdn.example/adsquared/2.ts")
        m = t.parse_media(text)
        self.assertEqual([s["ad"] for s in m["segments"]], [False, True])
        self.assertTrue(m["ads"])

    def test_cue_out_to_cue_in(self):
        lines = playlist(10, [(0, "live"), (2, "live"), (4, "live")]).splitlines()
        i = lines.index("https://cdn.example/live-0.ts")
        lines[i + 1:i + 1] = ["#EXT-X-CUE-OUT:DURATION=2"]
        j = lines.index("https://cdn.example/live-2.ts")
        lines[j + 1:j + 1] = ["#EXT-X-CUE-IN"]
        m = t.parse_media("\n".join(lines) + "\n")
        self.assertEqual([s["ad"] for s in m["segments"]], [False, True, False])

    def test_a_class_never_met_is_reported_and_cuts_nothing(self):
        tag = '#EXT-X-DATERANGE:ID="n-1",CLASS="twitch-something-new",START-DATE="2026-10-04T12:00:02.000Z",DURATION=4.000'
        m = t.parse_media(playlist(10, [(0, "live"), (2, "live")], tag))
        self.assertEqual(m["odd_classes"], ["twitch-something-new"])
        self.assertEqual([s["ad"] for s in m["segments"]], [False, False])

    def test_the_samples_hold_no_class_we_do_not_know(self):
        for f in sorted(glob.glob(os.path.join(SAMPLES, "*media*.m3u8"))):
            self.assertEqual(t.parse_media(open(f).read())["odd_classes"], [], f)


class Fmp4Tests(unittest.TestCase):
    """Channels Twitch sends as fMP4: the init piece (#EXT-X-MAP) must reach the player, or nothing decodes."""
    RAW = ("#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:100\n"
           '#EXT-X-MAP:URI="https://a.hls.ttvnw.net/v1/segment/INIT1.mp4"\n'
           "#EXT-X-PROGRAM-DATE-TIME:2026-10-08T14:00:00.000Z\n#EXTINF:2.000,live\nhttps://a.hls.ttvnw.net/v1/segment/S100.mp4\n"
           "#EXT-X-PROGRAM-DATE-TIME:2026-10-08T14:00:02.000Z\n#EXTINF:2.000,live\nhttps://a.hls.ttvnw.net/v1/segment/S101.mp4\n"
           '#EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI="https://a.hls.ttvnw.net/v1/segment/INIT2.mp4"\n'
           "#EXT-X-PROGRAM-DATE-TIME:2026-10-08T14:00:04.000Z\n#EXTINF:2.000,live\nhttps://a.hls.ttvnw.net/v1/segment/S102.mp4\n")

    def test_each_segment_knows_its_init_piece(self):
        m = t.parse_media(self.RAW)
        self.assertEqual([s["map"].rsplit("/", 1)[1] for s in m["segments"]], ["INIT1.mp4", "INIT1.mp4", "INIT2.mp4"])

    def test_the_playlist_handed_on_names_it_before_the_segments_it_serves(self):
        line = t.Timeline()
        line.feed("a", t.parse_media(self.RAW))
        out = line.render().splitlines()
        self.assertIn("#EXT-X-VERSION:6", out)
        maps = [i for i, l in enumerate(out) if l.startswith("#EXT-X-MAP:")]
        self.assertEqual(len(maps), 2)
        self.assertLess(maps[0], out.index("https://a.hls.ttvnw.net/v1/segment/S100.mp4"))
        self.assertTrue(out.index("https://a.hls.ttvnw.net/v1/segment/S101.mp4") < maps[1] < out.index("https://a.hls.ttvnw.net/v1/segment/S102.mp4"))

    def test_an_old_style_channel_is_written_as_before(self):
        line = t.Timeline()
        line.feed("a", t.parse_media(playlist(10, [(0, "live"), (2, "live")])))
        out = line.render()
        self.assertIn("#EXT-X-VERSION:3", out)
        self.assertNotIn("#EXT-X-MAP", out)


class TimelineTests(unittest.TestCase):
    def urls(self, line):
        return [l for l in line.render().splitlines() if l.startswith("https://")]

    def seq(self, line):
        return int([l for l in line.render().splitlines() if l.startswith("#EXT-X-MEDIA-SEQUENCE")][0].split(":")[1])

    def test_midroll_is_cut_and_numbering_never_goes_back(self):
        line = t.Timeline(keep=8)
        # the window slides two seconds at a time through: stream, a 6 s advert, stream again
        story = [(s, "live") for s in range(0, 12, 2)] + [(s, "Amazon|1") for s in range(12, 18, 2)] + \
                [(s, "live") for s in range(18, 30, 2)]
        last_seq = -1
        for start in range(0, len(story) - 4):
            line.feed("s1", t.parse_media(playlist(100 + start, story[start:start + 5])))
            urls = self.urls(line)
            self.assertFalse(any("ad-" in u for u in urls))
            self.assertGreaterEqual(self.seq(line), last_seq)
            last_seq = self.seq(line)
        self.assertEqual(line.cut, 3)
        self.assertEqual(line.passed, 12)
        text = line.render()
        self.assertEqual(text.count("#EXT-X-DISCONTINUITY\n"), 1)
        self.assertLess(text.index("live-10.ts"), text.index("#EXT-X-DISCONTINUITY\n"))
        self.assertLess(text.index("#EXT-X-DISCONTINUITY\n"), text.index("live-18.ts"))

    def test_playlist_holds_still_while_only_adverts_arrive(self):
        line = t.Timeline()
        line.feed("s1", t.parse_media(playlist(1, [(0, "live"), (2, "live")])))
        held = line.render()
        for i in range(3):
            line.feed("s1", t.parse_media(playlist(3 + i, [(4 + 2 * i, "DCM|7"), (6 + 2 * i, "DCM|7")])))
            self.assertEqual(line.render(), held)

    def test_same_playlist_twice_adds_nothing(self):
        line = t.Timeline()
        p = t.parse_media(playlist(1, [(0, "live"), (2, "live")]))
        self.assertEqual(line.feed("s1", p), (2, 0))
        self.assertEqual(line.feed("s1", p), (0, 0))

    def test_new_session_overlapping_in_time_is_not_repeated(self):
        line = t.Timeline()
        line.feed("s1", t.parse_media(playlist(50, [(0, "live"), (2, "live"), (4, "live")])))
        # a new session numbers from somewhere else and its window overlaps the old one
        line.feed("s2", t.parse_media(playlist(900, [(2, "live"), (4, "live"), (6, "live"), (8, "live")])))
        self.assertEqual([u.rsplit("-", 1)[1] for u in self.urls(line)], ["0.ts", "2.ts", "4.ts", "6.ts", "8.ts"])
        text = line.render()
        self.assertEqual(text.count("#EXT-X-DISCONTINUITY\n"), 1)
        self.assertLess(text.index("#EXT-X-DISCONTINUITY\n"), text.index("live-6.ts"))

    def test_discontinuity_sequence_counts_what_slid_out(self):
        line = t.Timeline(keep=2)
        line.feed("s1", t.parse_media(playlist(1, [(0, "live"), (2, "Amazon|1"), (4, "live"), (6, "live"), (8, "live")])))
        text = line.render()
        # live-4 began a new run and has slid out of view: one discontinuity is behind us
        self.assertIn("#EXT-X-DISCONTINUITY-SEQUENCE:1", text)
        self.assertNotIn("#EXT-X-DISCONTINUITY\n", text)
        self.assertEqual(self.seq(line), 2)


class Upstream(unittest.TestCase):
    """upstream.py: the downloaded copies of other projects' code in upstream/ hold nothing twitchlib does not account for."""

    def test_saved_copies_hold_nothing_new(self):
        import upstream
        if not all(os.path.exists(os.path.join(upstream.RESEARCH, n)) for n in upstream.SOURCES):
            self.skipTest("the upstream copies are not downloaded (python3 upstream.py fetches them)")
        files = {n: open(os.path.join(upstream.RESEARCH, n), encoding="utf-8", errors="replace").read() for n in upstream.SOURCES}
        found = upstream.facts(files)
        self.assertEqual(upstream.compare(found), [])
        self.assertIn("twitch-stitched", found["vaft advert marks"])
        self.assertEqual(found["vaft client id"], t.CLIENT_ID)

    def test_a_new_mark_is_reported(self):
        import upstream
        vaft = "// @version 99\nscope.AdSignifiers = ['stitched-ad', 'twitch-brand-new'];\nscope.ClientID = 'other';\n"
        lines = [l for _, l in upstream.compare(upstream.facts({"vaft.user.js": vaft}))]
        self.assertTrue(any("twitch-brand-new" in l for l in lines), lines)
        self.assertTrue(any("client id is other" in l for l in lines), lines)
        self.assertFalse(any("'stitched-ad'" in l for l in lines), lines)


if __name__ == "__main__":
    unittest.main()
