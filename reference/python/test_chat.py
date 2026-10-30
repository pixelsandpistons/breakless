#!/usr/bin/env python3
"""Tests for chat.py. No network: python3 twitch/test_chat.py

`python3 twitch/test_chat.py --live [channel]` is the one check that does use the network: it joins a busy
channel's chat for 20 seconds and asks the three emote services, and prints what came back.
"""
import sys
import threading
import time
import unittest
import urllib.error

import os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import chat  # noqa: E402

MSG = ("@badge-info=subscriber/14;badges=moderator/1,subscriber/12,bits/1000;color=#1E90FF;display-name=Some\\sOne;"
       "emotes=25:6-10,18-22/1902:12-16;first-msg=0;id=abc-1;room-id=71092938;tmi-sent-ts=1;user-id=7 "
       ":someone!someone@someone.tmi.twitch.tv PRIVMSG #chan :hello Kappa Keepo Kappa: done")


class ParseTests(unittest.TestCase):
    def test_tags_are_unescaped(self):
        tags, prefix, command, params = chat.parse_line(MSG)
        self.assertEqual(tags["display-name"], "Some One")
        self.assertEqual((prefix.split("!")[0], command, params[0]), ("someone", "PRIVMSG", "#chan"))
        self.assertEqual(params[1], "hello Kappa Keepo Kappa: done")      # a colon inside the text stays

    def test_message_event(self):
        ev = chat.event(*chat.parse_line(MSG))
        self.assertEqual((ev["k"], ev["login"], ev["name"], ev["color"], ev["id"]), ("m", "someone", "Some One", "#1E90FF", "abc-1"))
        self.assertEqual(ev["emotes"], [["25", 6, 10], ["1902", 12, 16], ["25", 18, 22]])
        self.assertEqual(ev["badges"], ["moderator", "subscriber"])      # bits is not a badge we show
        self.assertEqual([ev["text"][a:b + 1] for _, a, b in ev["emotes"]], ["Kappa", "Keepo", "Kappa"])

    def test_action_moves_the_emote_positions(self):
        line = "@emotes=25:8-12;display-name=A;id=x :a!a@a PRIVMSG #chan :\x01ACTION Kappa\x01"
        ev = chat.event(*chat.parse_line(line))
        self.assertTrue(ev["me"])
        self.assertEqual((ev["text"], ev["emotes"]), ("Kappa", [["25", 0, 4]]))

    def test_a_colour_that_is_not_one_is_dropped(self):
        ev = chat.event(*chat.parse_line("@color=red\\:url(x);display-name=A :a!a@a PRIVMSG #c :hi"))
        self.assertEqual(ev["color"], "")

    def test_notice_and_removals(self):
        ev = chat.event(*chat.parse_line("@system-msg=A\\ssubscribed\\swith\\sPrime.;msg-id=sub :tmi.twitch.tv USERNOTICE #c :thanks"))
        self.assertEqual(ev, {"k": "n", "text": "A subscribed with Prime.", "said": "thanks"})
        self.assertEqual(chat.event(*chat.parse_line("@ban-duration=600 :tmi.twitch.tv CLEARCHAT #c :rude")), {"k": "x", "login": "rude"})
        self.assertEqual(chat.event(*chat.parse_line(":tmi.twitch.tv CLEARCHAT #c")), {"k": "x", "login": ""})
        self.assertEqual(chat.event(*chat.parse_line("@target-msg-id=abc-1 :tmi.twitch.tv CLEARMSG #c :hello")), {"k": "d", "id": "abc-1"})
        self.assertIsNone(chat.event(*chat.parse_line(":tmi.twitch.tv 001 justinfan1 :Welcome, GLHF!")))

    def test_bad_emote_specs(self):
        self.assertEqual(chat.parse_emotes(""), [])
        self.assertEqual(chat.parse_emotes("25:9-2/../x:1/emotesv2_ab12:0-3"), [["emotesv2_ab12", 0, 3]])


class RoomTests(unittest.TestCase):
    def room(self):
        return chat.Room("chan", connect=False)

    def test_ping_roomstate_and_numbering(self):
        r = self.room()
        self.assertEqual(r.take(*chat.parse_line("PING :tmi.twitch.tv")), "PONG :tmi.twitch.tv")
        r.take(*chat.parse_line("@room-id=71092938;slow=0 :tmi.twitch.tv ROOMSTATE #chan"))
        self.assertEqual((r.room_id, r.state), ("71092938", "joined"))
        for _ in range(3):
            r.take(*chat.parse_line(MSG))
        got = r.poll(None)
        self.assertEqual([e["n"] for e in got["events"]], [1, 2, 3])
        self.assertEqual((got["seq"], got["room_id"]), (3, "71092938"))
        self.assertEqual([e["n"] for e in r.poll(2)["events"]], [3])
        self.assertEqual(r.poll(3)["events"], [])

    def test_poll_waits_for_the_next_message(self):
        r = self.room()
        threading.Timer(0.2, lambda: r.take(*chat.parse_line(MSG))).start()
        t0 = time.time()
        got = r.poll(0, wait=5)
        self.assertEqual(len(got["events"]), 1)
        self.assertLess(time.time() - t0, 2)

    def test_poll_gives_up_after_wait(self):
        t0 = time.time()
        self.assertEqual(self.room().poll(0, wait=0.3)["events"], [])
        self.assertGreaterEqual(time.time() - t0, 0.25)

    def test_a_number_from_before_a_restart_gets_the_recent_ones(self):
        r = self.room()
        r.take(*chat.parse_line(MSG))
        self.assertEqual(len(r.poll(5000)["events"]), 1)

    def test_only_the_last_events_are_kept(self):
        r = self.room()
        for _ in range(chat.KEEP + 30):
            r.take(*chat.parse_line(MSG))
        self.assertEqual(len(r.events), chat.KEEP)
        self.assertEqual(len(r.poll(None)["events"]), 60)

    def test_reconnect_is_an_error_the_loop_handles(self):
        with self.assertRaises(ConnectionError):
            self.room().take(*chat.parse_line(":tmi.twitch.tv RECONNECT"))


BTTV_GLOBAL = [{"id": "54fa8f1401e468494b85b537", "code": ":tf:", "imageType": "png", "modifier": False},
               {"id": "bad id!", "code": "Nope"}]
BTTV_CHANNEL = {"channelEmotes": [{"id": "aaa", "code": "catJAM"}], "sharedEmotes": [{"id": "bbb", "code": "monkaS"}]}
FFZ_GLOBAL = {"default_sets": [3], "sets": {"3": {"emoticons": [{"id": 9, "name": "ZrehplaR"}]},
                                            "4": {"emoticons": [{"id": 10, "name": "NotDefault"}]}}}
FFZ_ROOM = {"room": {}, "sets": {"166907": {"emoticons": [{"id": 77, "name": "catJAM"}, {"id": 78, "name": "OnlyFFZ"}]}}}
STV_GLOBAL = {"emotes": [{"id": "01A", "name": "reckH", "flags": 0}, {"id": "01B", "name": "RainTime", "flags": 1}]}
STV_USER = {"emote_set": {"emotes": [{"id": "01C", "name": "catJAM", "flags": 0}]}}


class EmoteTests(unittest.TestCase):
    def setUp(self):
        chat._cache.clear()

    def fetch(self, url):
        table = {"betterttv.net/3/cached/emotes/global": BTTV_GLOBAL, "betterttv.net/3/cached/users": BTTV_CHANNEL,
                 "frankerfacez.com/v1/set/global": FFZ_GLOBAL, "frankerfacez.com/v1/room": FFZ_ROOM,
                 "7tv.io/v3/emote-sets/global": STV_GLOBAL, "7tv.io/v3/users": STV_USER}
        return next(v for k, v in table.items() if k in url)

    def test_readers(self):
        self.assertEqual(chat.from_bttv(BTTV_GLOBAL), {":tf:": ["b", "54fa8f1401e468494b85b537", False]})
        self.assertEqual(chat.from_bttv(BTTV_CHANNEL), {"catJAM": ["b", "aaa", False], "monkaS": ["b", "bbb", False]})
        self.assertEqual(chat.from_ffz(FFZ_GLOBAL), {"ZrehplaR": ["f", "9", False]})
        self.assertEqual(set(chat.from_ffz(FFZ_ROOM)), {"catJAM", "OnlyFFZ"})
        self.assertEqual(chat.from_7tv(STV_GLOBAL), {"reckH": ["7", "01A", False], "RainTime": ["7", "01B", True]})

    def test_merge_channel_wins_and_7tv_wins_a_shared_word(self):
        got = chat.emotes("71092938", fetch=self.fetch)
        e = got["emotes"]
        self.assertEqual(e["catJAM"], ["7", "01C", False])
        self.assertEqual((e["OnlyFFZ"][0], e["monkaS"][0], e[":tf:"][0], e["ZrehplaR"][0]), ("f", "b", "b", "f"))
        self.assertEqual(got["counts"]["b"], {"global": 1, "channel": 2})
        self.assertEqual(got["errors"], {})

    def test_without_a_room_number_only_the_globals(self):
        got = chat.emotes("", fetch=self.fetch)
        self.assertNotIn("catJAM", got["emotes"])
        self.assertIn("reckH", got["emotes"])

    def test_one_provider_away_does_not_empty_the_rest(self):
        def fetch(url):
            if "7tv" in url:
                raise OSError("timed out")
            if "frankerfacez.com/v1/room" in url:
                raise urllib.error.HTTPError(url, 404, "no such room", {}, None)
            return self.fetch(url)
        got = chat.emotes("71092938", fetch=fetch)
        self.assertEqual(got["emotes"]["catJAM"], ["b", "aaa", False])
        self.assertEqual(sorted(got["errors"]), ["7TV channel", "7TV global"])      # the 404 is not a fault

    def test_answers_are_reused(self):
        calls = []
        def fetch(url):
            calls.append(url)
            return self.fetch(url)
        chat.emotes("71092938", fetch=fetch)
        chat.emotes("71092938", fetch=fetch)
        self.assertEqual(len(calls), 6)

    def test_image_addresses(self):
        self.assertEqual(chat.image_url("b", "abc"), "https://cdn.betterttv.net/emote/abc/2x.webp")
        self.assertEqual(chat.image_url("t", "emotesv2_ab12"), "https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_ab12/default/dark/2.0")
        for bad in (("x", "abc"), ("b", "../x"), ("b", ""), ("7", "a/b"), ("f", "a?b")):
            self.assertIsNone(chat.image_url(*bad))


def live(login):
    r = chat.Room(login)
    time.sleep(20)
    got = r.poll(None)
    r.close()
    print("%s: state %s, room %s, %d events in 20 s" % (login, got["state"], got["room_id"], got["seq"]))
    for e in got["events"][-3:]:
        print("  ", {k: e[k] for k in ("k", "name", "text", "emotes", "badges") if k in e})
    em = chat.emotes(got["room_id"])
    print("emotes: %d words, counts %s, errors %s" % (len(em["emotes"]), em["counts"], em["errors"]))
    return got["state"] == "joined" and bool(em["emotes"])


if __name__ == "__main__":
    if "--live" in sys.argv:
        rest = [a for a in sys.argv[1:] if a != "--live"]
        sys.exit(0 if live(rest[0] if rest else "xqc") else 1)
    unittest.main()
