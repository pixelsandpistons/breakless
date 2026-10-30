#!/usr/bin/env python3
"""Follow one channel's playlist exactly as Twitch sends it and print a line whenever the kinds of
tag or the segment titles change. This is how to see what Twitch is really doing when the service
seems to cut too much or too little.

    python3 twitch/watch_raw.py CHANNEL                 # from the home line, 5 minutes
    python3 twitch/watch_raw.py CHANNEL -m 10
    docker exec twitch python /app/watch_raw.py CHANNEL --proxy http://twitch-vpn:8888    # through the exit
    python3 twitch/watch_raw.py CHANNEL --dump          # also print each new advert-ish tag in full

Reading it: titles ['live'] is stream. 'Amazon' or 'DCM' are advert segments (class
twitch-stitched-ad comes with them): those are cut. Class twitch-maf-ad with titles still ['live']
is Twitch asking its own player to show an advert; the video is untouched and nothing is cut.
Anything else is new: capture it (capture.py) and teach twitchlib.parse_media about it.
"""
import re
import sys
import time

import twitchlib as t

argv = sys.argv[1:]
dump = "--dump" in argv
argv = [a for a in argv if a != "--dump"]
proxy = minutes = None
for flag in ("--proxy", "-m"):
    if flag in argv:
        i = argv.index(flag)
        if flag == "--proxy":
            proxy = argv[i + 1]
        else:
            minutes = float(argv[i + 1])
        del argv[i:i + 2]
via = t.opener(proxy)
login = argv[0]
master = t.master(login, via=via)
url = t.variants(master)[0]["url"]
print("Twitch sees us in %s; watching %s" % (t.master_info(master)["country"], login), flush=True)
start, seen, shown = time.time(), None, set()
while time.time() - start < (minutes or 5) * 60:
    text = t.http(url, via=via)
    classes = sorted(set(re.findall(r'CLASS="([^"]+)"', text)) - {"timestamp", "twitch-session", "twitch-stream-source", "twitch-trigger"})
    titles = sorted({l.split(",", 1)[1].split("|")[0] for l in text.splitlines() if l.startswith("#EXTINF")})
    if (classes, titles) != seen:
        seen = (classes, titles)
        cut = sum(1 for s in t.parse_media(text)["segments"] if s["ad"])
        print("t=%4ds  classes=%s  titles=%s  we would cut %d of %d segments" % (
            time.time() - start, classes or "none", titles, cut, len(t.parse_media(text)["segments"])), flush=True)
    if dump:
        for line in text.splitlines():
            m = re.match(r'#EXT-X-DATERANGE:ID="((?:stitched-ad|maf-ad)[^"]*)"', line)
            if m and m.group(1) not in shown:
                shown.add(m.group(1))
                print("   " + re.sub(r'="[^"]{80,}"', '="(long)"', line), flush=True)
    time.sleep(2)
