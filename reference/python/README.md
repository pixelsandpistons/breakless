# The Python prototype

This is the service as it has run at the maintainer's home since 2026-10-04, standard library only.
It is the specification for the Rust crates and the oracle for their differential tests. It leaves
the tree at 1.0; until then, fixes to it are welcome if they come with the same change in the Rust
code or a note that the Rust code does not have the behaviour yet.

| File | What |
|---|---|
| `server.py` | The service: sessions, fallback, routes, files. The docstring lists every route and environment variable |
| `twitchlib.py` | Everything that knows Twitch's formats: token request, playlist addresses, advert marks, `Timeline`. Ported to `crates/formats` |
| `chat.py` | Chat as a guest over IRC, events, emote lists. Task 01 ports the pure parts |
| `probe.py` | Pre-roll measurement, per token kind, straight out or through a proxy |
| `soak.py` | Watches channels through the service for a while and reports breaks and stalls |
| `watch_raw.py` | Follows one channel's playlist as Twitch sends it and prints every change of tags and titles |
| `capture.py` | Saves a scrubbed master and media playlist pair into `fixtures/playlists/` |
| `upstream.py` | What other advert-cutting projects look for that this does not. Task 02 ports it |
| `test_twitchlib.py`, `test_chat.py` | 23 and 18 tests, no network |

```sh
python3 reference/python/test_twitchlib.py
python3 reference/python/test_chat.py
python3 reference/python/test_chat.py --live xqc        # joins a real chat for 20 s
```

Run it by hand on loopback, without a VPN, to see adverts being cut:

```sh
mkdir -p /tmp/breakless-data
cd reference/python
TWITCH_BIND=127.0.0.1 TWITCH_PROXY= TWITCH_DATA=/tmp/breakless-data TWITCH_PORT=18080 python3 -u server.py &
python3 soak.py -m 5 -b http://127.0.0.1:18080
cat /tmp/breakless-data/adverts.jsonl
kill %1
```

The page under `web/` is served from `static/` beside `server.py`; the compose file in `deploy/`
mounts it there. For a hand-run copy, `ln -s ../../web static` once.
