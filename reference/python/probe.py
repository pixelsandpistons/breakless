#!/usr/bin/env python3
"""Measure where Twitch serves adverts right now.

    python3 twitch/probe.py                       # from this machine, straight out
    python3 twitch/probe.py -n 12                 # more channels (default 6, the most watched)
    python3 twitch/probe.py somechannel other     # named channels
    python3 twitch/probe.py --types               # every token type, not only "site"
    python3 twitch/probe.py --random              # 6 of the 30 most watched, different each run
    python3 twitch/probe.py --proxy http://twitch-vpn:8888     # through an HTTP proxy
    python3 twitch/probe.py --json                # the same as data

Through the service's own exit (the usual way; the proxy is only reachable inside Docker):

    docker exec twitch python /app/probe.py --proxy http://twitch-vpn:8888

A fresh token is what a viewer gets on opening a stream, so this measures pre-roll adverts.
Mid-roll adverts only show while watching; the service counts those itself (README.md section 5).
The last line, "clean / answered", is the answer: 6/6 means no adverts on six channels.
"""
import json
import random
import sys
import time

import twitchlib as t


def probe(login, types, via):
    row = {}
    dev = t.device_id()
    for ptype, platform in types:
        try:
            m = t.master(login, ptype, platform, dev, via)
            v = t.variants(m)
            if not v:
                row[ptype] = {"error": "no variants"}
                continue
            media = t.parse_media(t.http(v[0]["url"], via=via))
            ads = media["ads"]
            row[ptype] = {"ads": bool(ads), "roll": ads["roll"] if ads else "", "seconds": ads["seconds"] if ads else "",
                          "best": v[0]["name"], "qualities": len(v), "country": t.master_info(m)["country"],
                          "cluster": t.master_info(m)["cluster"], "odd_titles": media["odd_titles"]}
        except t.TwitchError as e:
            row[ptype] = {"error": str(e)[:120]}
    return row


def main(argv):
    def flag(name):
        if name in argv:
            argv.remove(name)
            return True
        return False

    def value(name, default=None):
        if name in argv:
            i = argv.index(name)
            v = argv[i + 1]
            del argv[i:i + 2]
            return v
        return default

    as_json = flag("--json")
    types = t.PLAYER_TYPES if flag("--types") else t.PLAYER_TYPES[:1]
    n = int(value("-n", 6))
    proxy = value("--proxy")
    via = t.opener(proxy)
    # --random: n of the 30 most watched, so repeated runs do not keep asking the same channels
    # (Twitch shows a pre-roll once, then not again for some minutes, which would hide a change)
    pick_random = flag("--random")
    logins = argv or [s["login"] for s in (random.sample(t.top(30), n) if pick_random else t.top(n))]
    result = {"when": time.strftime("%Y-%m-%d %H:%M:%S %z"), "proxy": bool(proxy), "channels": {}}
    for login in logins:
        result["channels"][login] = probe(login, types, via)
    if as_json:
        json.dump(result, sys.stdout, indent=1)
        print()
        return 0
    names = [p for p, _ in types]
    print("%-20s" % "channel" + "".join("%-22s" % p for p in names))
    tally = {p: [0, 0] for p in names}
    seen = set()
    for login, row in result["channels"].items():
        cells = []
        for p in names:
            r = row.get(p, {})
            if "error" in r:
                cells.append("error")
                continue
            tally[p][1] += 1
            tally[p][0] += 0 if r["ads"] else 1
            seen.add((r["country"], r["cluster"]))
            cells.append(("ADS %s %ss" % (r["roll"], r["seconds"]) if r["ads"] else "clean") + " " + r["best"].split(" ")[0])
        print("%-20s" % login[:19] + "".join("%-22s" % c[:21] for c in cells))
    print("%-20s" % "clean / answered" + "".join("%-22s" % ("%d/%d" % tuple(tally[p])) for p in names))
    print("Twitch saw us in: " + ", ".join(sorted("%s (%s)" % s for s in seen)))
    for p, e in sorted({(p, r["error"]) for row in result["channels"].values() for p, r in row.items() if "error" in r}):
        print("  error %s: %s" % (p, e))
    odd = sorted({x for row in result["channels"].values() for r in row.values() for x in r.get("odd_titles", [])})
    if odd:
        print("  segment titles not seen before (twitchlib.py, STREAM_TITLES): %s" % odd)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
