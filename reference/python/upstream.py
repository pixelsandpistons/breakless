#!/usr/bin/env python3
"""Staying ahead: what do the other advert-cutting projects know today that twitchlib.py does not?

    python3 reference/python/upstream.py            fetch into upstream/ (ignored by Git), compare, print; exit 1 when something needs a look
    python3 reference/python/upstream.py --offline  compare the copies already in upstream/ (no network)

Twitch changes how it marks adverts a few times a year. The projects below are used by far more people than this
service, so a new mark shows up in their code days before it costs us a stream. This script downloads their
current source into upstream/ (ignored by Git), pulls out the few facts that matter and
checks each against twitchlib.py:

    vaft (ryanbr/TwitchAdSolutions)   the strings that mean "advert" in a playlist, the DATERANGE classes that do not,
                                      the segment addresses that are adverts, the client id, the token kinds it falls back to
    Streamlink's Twitch plugin        how it tells an advert DATERANGE and an advert segment, the client id

Every fact is either ACCOUNTED for below (with what twitchlib does about it) or reported as NEW. A NEW line is not
a fault yet: read the upstream change (the script prints where), decide, change twitchlib.py and its tests, then add
the fact to ACCOUNTED here. PLAN.md, "Staying ahead", is the routine; upstream/upstream-state.json remembers what was
seen last time so a version change is reported once.

Standard library only. Runs on the host (it asks GitHub, not Twitch).
"""
import hashlib
import json
import os
import re
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
RESEARCH = os.environ.get("BREAKLESS_UPSTREAM_DIR", os.path.join(HERE, "..", "..", "upstream"))
STATE = os.path.join(RESEARCH, "upstream-state.json")
sys.path.insert(0, HERE)
import twitchlib as t  # noqa: E402

SOURCES = {
    "vaft.user.js": "https://raw.githubusercontent.com/ryanbr/TwitchAdSolutions/master/vaft/vaft.user.js",
    "streamlink-twitch-plugin.py": "https://raw.githubusercontent.com/streamlink/streamlink/master/src/streamlink/plugins/twitch.py",
    "ryanbr-issues.md": "https://raw.githubusercontent.com/ryanbr/TwitchAdSolutions/master/issues.md",
    "ryanbr-full-list.md": "https://raw.githubusercontent.com/ryanbr/TwitchAdSolutions/master/full-list.md",
    "ttv-lol-pro-README.md": "https://raw.githubusercontent.com/younesaassila/ttv-lol-pro/v2/README.md",
}

# What twitchlib.py does about each thing the others look for. Add a line here only after twitchlib and its tests
# have been changed (or after deciding, with the reason, that nothing needs to change).
ACCOUNTED = {
    # vaft AdSignifiers
    "stitched-ad": "cut: AD_DATERANGE matches ID=\"stitched-ad-",
    "twitch-stitched": "cut: AD_DATERANGE matches the whole CLASS=\"twitch-stitched family",
    "EXT-X-CUE-OUT": "cut: parse_media marks everything up to EXT-X-CUE-IN",
    'EXT-X-DATERANGE:CLASS="twitch-maf-ad"': "not cut, counted: an advert the official player draws itself (CLIENT_SIDE); the segments are the stream",
    'EXT-X-DATERANGE:CLASS="twitch-trigger"': "not cut: in every clean sample here (KNOWN_CLASSES); vaft uses it to fetch a backup early",
    # vaft AdSegmentURLPatterns
    "/adsquared/": "cut: AD_URLS",
    "/_404/": "cut: AD_URLS",
    "/processing": "not cut: too loose a word to cut a segment on by itself; an advert under it is still caught by title or window",
    # vaft KnownNonAdSignifiers
    "twitch-session": "known: KNOWN_CLASSES", "twitch-stream-source": "known: KNOWN_CLASSES",
    "twitch-ad-quartile": "known: KNOWN_CLASSES", "twitch-assignment": "known: KNOWN_CLASSES",
    # Streamlink
    "streamlink:class:twitch-stitched-ad": "cut: AD_DATERANGE",
    "streamlink:id:stitched-ad-": "cut: AD_DATERANGE",
    "streamlink:title:Amazon": "cut: parse_media's known_ad",
}


def js_list(text, name):
    """The strings of `scope.<name> = [ ... ];` in vaft, comments left out."""
    m = re.search(r"scope\.%s\s*=\s*\[(.*?)\];" % re.escape(name), text, re.S)
    if not m:
        return None
    body = re.sub(r"//[^\n]*", "", m.group(1))
    return [a or b for a, b in re.findall(r"'((?:[^'\\]|\\.)*)'|\"((?:[^\"\\]|\\.)*)\"", body)]


def facts(files):
    """{name: value} from the downloaded sources; a value of None means the place it was read from is gone."""
    vaft, sl = files.get("vaft.user.js", ""), files.get("streamlink-twitch-plugin.py", "")
    version = re.search(r"@version\s+(\S+)", vaft)
    client = re.search(r"scope\.ClientID\s*=\s*'([^']+)'", vaft)
    ad_fn = re.search(r"def _is_daterange_ad\(.*?\n(.*?)\n\n", sl, re.S)
    seg_fn = re.search(r"def _is_segment_ad\(.*?\n(.*?)\n\n", sl, re.S)
    sl_marks = None
    if ad_fn and seg_fn:
        sl_marks = (["streamlink:class:" + x for x in re.findall(r'classname == "([^"]+)"', ad_fn.group(1))]
                    + ["streamlink:id:" + x for x in re.findall(r'startswith\("([^"]+)"\)', ad_fn.group(1))]
                    + ["streamlink:title:" + x for x in re.findall(r'"([^"]+)" in title', seg_fn.group(1))])
    sl_client = re.search(r'CLIENT_ID\s*=\s*"([a-z0-9]{20,40})"', sl)
    return {
        "vaft version": version.group(1) if version else None,
        "vaft advert marks": js_list(vaft, "AdSignifiers"),
        "vaft classes that are not adverts": js_list(vaft, "KnownNonAdSignifiers"),
        "vaft advert segment addresses": js_list(vaft, "AdSegmentURLPatterns"),
        "vaft backup token kinds": js_list(vaft, "BackupPlayerTypes"),
        "vaft client id": client.group(1) if client else None,
        "streamlink advert marks": sl_marks,
        "streamlink client id": sl_client.group(1) if sl_client else None,
    }


def compare(found):
    """[(severity, line)] where severity is NEW (look now), GONE (the place we read moved: fix this script) or note."""
    out = []
    for name, value in found.items():
        if value is None:
            out.append(("GONE", "%s: not found where it used to be; the source was reshaped, read it and fix upstream.py" % name))
    for name in ("vaft advert marks", "vaft classes that are not adverts", "vaft advert segment addresses", "streamlink advert marks"):
        for mark in found.get(name) or []:
            if mark not in ACCOUNTED:
                out.append(("NEW", "%s: %r is not accounted for in twitchlib.py" % (name, mark)))
    for name in ("vaft client id", "streamlink client id"):
        if found.get(name) and found[name] != t.CLIENT_ID:
            out.append(("NEW", "%s is %s, ours is %s" % (name, found[name], t.CLIENT_ID)))
    ours = {p for p, _ in t.PLAYER_TYPES}
    for kind in found.get("vaft backup token kinds") or []:
        if kind not in ours:
            out.append(("NEW", "vaft falls back to token kind %r, which probe.py does not measure (PLAYER_TYPES)" % kind))
    return out


def main():
    offline = "--offline" in sys.argv
    os.makedirs(RESEARCH, exist_ok=True)
    files, changed = {}, []
    try:
        state = json.load(open(STATE))
    except (OSError, ValueError):
        state = {}
    for name, url in SOURCES.items():
        path = os.path.join(RESEARCH, name)
        if not offline:
            try:
                with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "breakless-upstream"}), timeout=30) as r:
                    data = r.read()
                if len(data) < 500:
                    raise OSError("only %d bytes" % len(data))
                with open(path, "wb") as f:
                    f.write(data)
            except OSError as e:
                print("could not fetch %s: %s (using the copy in upstream/)" % (name, e))
        try:
            files[name] = open(path, encoding="utf-8", errors="replace").read()
        except OSError:
            files[name] = ""
        digest = hashlib.sha256(files[name].encode()).hexdigest()[:16]
        if state.get("files", {}).get(name) not in (None, digest):
            changed.append(name)
        state.setdefault("files", {})[name] = digest
    found = facts(files)
    before = state.get("facts") or {}
    print("vaft %s; %d advert marks, %d advert addresses, %d harmless classes; backup token kinds %s" % (
        found["vaft version"], len(found["vaft advert marks"] or []), len(found["vaft advert segment addresses"] or []),
        len(found["vaft classes that are not adverts"] or []), ", ".join(found["vaft backup token kinds"] or [])))
    for name in changed:
        print("changed since the last look: research/%s  (git diff -- twitch/research/%s shows what)" % (name, name))
    for name, value in found.items():
        if name in before and before[name] != value:
            print("was: %s = %s\nnow: %s = %s" % (name, json.dumps(before[name]), name, json.dumps(value)))
    problems = compare(found)
    for severity, line in problems:
        print("%-4s %s" % (severity, line))
    if not offline:
        state.update(facts=found, looked=time.strftime("%Y-%m-%d %H:%M"))
        with open(STATE, "w") as f:
            json.dump(state, f, indent=1, sort_keys=True)
    if not problems:
        print("nothing new: everything the others look for is accounted for in twitchlib.py")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
