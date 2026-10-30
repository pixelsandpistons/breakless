#!/usr/bin/env python3
"""Watch several channels through the service for a while, the way players would, and report
every advert break it met. This is how mid-roll adverts are measured (probe.py only sees pre-rolls).

    python3 twitch/soak.py                      # 6 most-watched channels, 20 minutes, the service at $TWITCH_BASE (default http://127.0.0.1:8080)
    python3 twitch/soak.py -m 60 chan1 chan2    # named channels, an hour
    python3 twitch/soak.py -b http://127.0.0.1:18080     # a copy started by hand (README.md section 5)
    docker exec twitch python /app/soak.py -m 25        # the live service, from inside its container

Each channel is polled every 2 s by its own thread. Only playlists are fetched (a few kB), no video.
Columns: from/seen = where its playlists are asked from now and the country Twitch sees; passed/cut =
stream segments handed on and advert segments cut; stalled_s = seconds in which the playlist did not
move for more than 10 s at a stretch (what a viewer would see as a frozen picture); errors = requests
that failed (a channel that went offline shows many).
The breaks themselves, with times, are in the service's adverts.jsonl.
"""
import json
import os
import sys
import threading
import time
import urllib.request

argv = sys.argv[1:]


def value(name, default):
    if name in argv:
        i = argv.index(name)
        v = argv[i + 1]
        del argv[i:i + 2]
        return v
    return default


base = value("-b", os.environ.get("TWITCH_BASE", "http://127.0.0.1:8080")).rstrip("/")
minutes = float(value("-m", 20))
count = int(value("-n", 6))


def get(path):
    with urllib.request.urlopen(base + path, timeout=30) as r:
        return r.read().decode()


channels = argv or [c["login"] for c in json.loads(get("/api/channels"))["top"][:count]]
end = time.time() + minutes * 60
errors = {c: 0 for c in channels}
stalls = {c: 0.0 for c in channels}


def watch(c):
    newest, since = None, time.time()
    while time.time() < end:
        try:
            tail = get("/live/%s/chunked.m3u8" % c).rstrip().rsplit("\n", 1)[-1]
            if tail != newest:
                if newest is not None and time.time() - since > 10:
                    stalls[c] += time.time() - since
                newest, since = tail, time.time()
        except Exception:
            errors[c] += 1
        time.sleep(2)
    if time.time() - since > 10:
        stalls[c] += time.time() - since


threads = [threading.Thread(target=watch, args=(c,)) for c in channels]
for th in threads:
    th.start()
for th in threads:
    th.join()
state = json.loads(get("/api/status"))
status = {w["channel"]: w for w in state["watching"]}
print("%-20s %-5s %-4s %8s %6s %9s %6s" % ("channel", "from", "seen", "passed", "cut", "stalled_s", "errors"))
for c in channels:
    w = status.get(c.lower(), {})
    print("%-20s %-5s %-4s %8s %6s %9.0f %6d" % (c[:19], w.get("from"), w.get("country"), w.get("segments_passed"),
                                              w.get("segments_cut"), stalls[c], errors[c]))
print(json.dumps(state["totals"]))
