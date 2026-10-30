# Architecture

## Wiring

```
player ──GET /live/c.m3u8──▶ service ──▶ exit (HTTP proxy in an advert-free country) ──▶ Twitch GraphQL: token
       ──GET /live/c/q.m3u8─▶   │                                                   ──▶ usher: master playlist
                                │                                                   ──▶ media playlist, every 0.9 s
                                │  exit down or failing twice: the same, straight out
                                ▼
                        Timeline per (channel, quality): stream segments only, our numbering
player ◀── the rebuilt playlist; the player fetches the segments from Twitch itself
```

A **session** is one token and one master playlist for one channel: where it was asked from (an
exit, or home), the country Twitch placed it in, and the per-quality playlist addresses. A channel
nobody has asked for in 90 s is dropped. A session opened from home goes back to the exit after 30 s.

An **advert break** is the run of playlists whose newest segment is an advert. Its segments are cut;
the rebuilt playlist holds still; when the stream returns, a discontinuity marks the join. One line
per break goes to `adverts.jsonl`.

**Chat** is a guest IRC connection per channel being read, turned into numbered events that pages
collect by long poll. **Emotes** are three providers' lists merged, channel over global, cached.

## Crates

| Crate | Holds | Depends on |
|---|---|---|
| `breakless-formats` | Everything that knows Twitch's formats and nothing that does I/O: playlists, advert marks, `Timeline`, chat lines and events, emote readers, the GraphQL text. Every fact that can go stale lives here, covered by fixtures | standard library (serde for the chat and emote shapes, when they land) |
| `breakless` (service) | tokio, axum, reqwest through an HTTP proxy, rustls for chat, the routes, the session and exit logic, the files on disk | `breakless-formats` |
| `breakless-tools` | probe, soak, watch-raw, capture, upstream | `breakless-formats`, reqwest |

The boundary rule: if a test for it needs a socket, it is not in `formats`.

## Why a hand-written playlist parser

Twitch's playlists carry `#EXT-X-TWITCH-INFO`, `#EXT-X-TWITCH-PREFETCH` and `X-TV-TWITCH-*` attributes
on `DATERANGE` lines, and the service must report any `DATERANGE` class or segment title it has not
met before. General HLS crates drop unknown tags and attributes, which is exactly the information
this service lives on. The parser is about 150 lines (`crates/formats/src/playlist.rs`) and is held
to the prototype's behaviour by the differential test.

## Fixtures

`fixtures/playlists/` are real playlists captured by `reference/python/capture.py`, which replaces
the viewer's address, tokens, segment paths and session ids, and renames the channel. `fixtures/expected/`
is what the Python prototype produces for each one, written by `fixtures/gen-expected.py`. When
Twitch changes something, the order is: capture, read, add a test, change the parser, regenerate.
