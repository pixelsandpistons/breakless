#!/usr/bin/env python3
"""Save one master playlist and one media playlist of a live channel into fixtures/playlists/,
with everything personal or signed taken out (our address, tokens, segment paths).

    python3 reference/python/capture.py NAME [channel] [--proxy http://exit:8888]

NAME goes into the file names: samples/NAME-master.m3u8 and samples/NAME-media.m3u8.
The tests (test_twitchlib.py) read the samples that exist; add one when Twitch changes its format.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import twitchlib as t  # noqa: E402

args = sys.argv[1:]
proxy = None
if "--proxy" in args:
    i = args.index("--proxy")
    proxy = args[i + 1]
    del args[i:i + 2]
name = args[0]
via = t.opener(proxy)
login = args[1] if len(args) > 1 else t.top(3)[0]["login"]
master = t.master(login, via=via)
media = t.http(t.variants(master)[0]["url"], via=via)


def scrub(text):
    text = re.sub(r'USER-IP="[^"]*"', 'USER-IP="203.0.113.7"', text)
    text = re.sub(r'(SERVING-ID|VIDEO-SESSION-ID|BROADCAST-ID)="[^"]*"', r'\1="0"', text)
    text = re.sub(r'\b([A-Z])="[A-Za-z0-9_=-]{40,}"', r'\1="(removed)"', text)
    text = re.sub(r"(https://[^/\s\"]+/)[^\s\"]{30,}", lambda m: m.group(1) + "(removed)", text)
    return text.replace(login, "somechannel")


out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "fixtures", "playlists")
for kind, text in (("master", master), ("media", media)):
    with open(os.path.join(out, "%s-%s.m3u8" % (name, kind)), "w") as f:
        f.write(scrub(text))
print(name, login, "adverts" if t.has_ads(media) else "clean", t.master_info(master)["country"])
