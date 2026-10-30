#!/usr/bin/env python3
"""Writes fixtures/expected/ from the Python implementation the Rust crate replaces.

    python3 fixtures/gen-expected.py            # reads reference/python/twitchlib.py

Run it once per new playlist fixture, while the Python service still exists; commit the output. The Rust
differential test (crates/formats/tests/differential.rs) then holds the port to it byte for byte.
"""
import glob
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "reference", "python"))
import twitchlib as t  # noqa: E402

out = os.path.join(HERE, "expected")
os.makedirs(out, exist_ok=True)
for path in sorted(glob.glob(os.path.join(HERE, "playlists", "*.m3u8"))):
    name = os.path.basename(path)[:-5]
    text = open(path).read()
    facts = []
    if name.endswith("-media"):
        m = t.parse_media(text)
        line = t.Timeline()
        added, cut = line.feed("s1", m)
        with open(os.path.join(out, name + ".render.m3u8"), "w") as f:
            f.write(line.render())
        a = m["ads"]
        facts += ["seq=%d" % m["seq"], "target=%d" % m["target"], "segments=%d" % len(m["segments"]),
                  "ads=" + ("%s|%s|%s" % (a["roll"], a["pod"], a["seconds"]) if a else "none"),
                  "added=%d" % added, "cut=%d" % cut, "client_side=" + str(m["client_side"]).lower(),
                  "odd_titles=" + ",".join(m["odd_titles"]), "odd_classes=" + ",".join(m["odd_classes"])]
    else:
        info = t.master_info(text)
        facts += ["country=" + info["country"], "cluster=" + info["cluster"]]
        facts += ["variant=%s|%s|%d|%s|%s|%s" % (v["slug"], v["name"], v["bandwidth"], v["resolution"], v["fps"], v["codecs"])
                  for v in t.variants(text)]
    with open(os.path.join(out, name + ".facts"), "w") as f:
        f.write("\n".join(facts) + "\n")
    print(name, *facts[:4])
