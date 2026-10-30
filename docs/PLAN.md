# Plan

What exists, what comes next and in which order, and what has to be decided. Dates are when a thing
was checked, not promises. The maintainer's working notes that produced this plan are private; the
reasoning that matters is here.

## Where it stands (2026-10-30)

- The Python prototype (`reference/python/`) has run at the maintainer's home since 2026-10-04,
  through a VPN exit in Iceland. In every soak and status read since, 0 advert breaks were met through
  the exit; from the maintainer's home line without the exit, every stream opened with a pre-roll of
  15 to 51 s and busy channels had mid-rolls of 1 to 3 minutes, all cut.
- `breakless-formats` parses the playlists and rebuilds the timeline exactly as the prototype does,
  proven by `crates/formats/tests/differential.rs` against `fixtures/expected/`.
- Nothing else is written in Rust yet.

## Phases

### 1. Formats (done except chat)

- [x] Master and media playlists, advert marks, unknown-mark reporting, `Timeline`.
- [ ] Chat: IRC line and tag parsing, events, the room state machine, emote list readers and merge
      ([task 1](tasks/01-chat-formats.md)).
- [ ] Twitch request and response types for the GraphQL calls the service makes (token, who is live,
      most watched).

### 2. Service

The axum binary. Contract: every route and JSON field listed at the top of
`reference/python/server.py`, and the playlists byte for byte.

- [x] Sessions and exits. Exits are a list; each has its own rest timer after a failure; a channel
      opens through the first usable exit, tries it twice, then the other exits, then the home line,
      and moves back when an exit is usable again. Each quality refreshes under its own lock, so
      one slow quality does not hold the others; opening a session holds the channel's lock.
- [x] `/api/status`, `/api/watch`, `/api/channels`, `/api/follow`, static files, the CSP header.
- [x] `adverts.jsonl` (same line shape, 1 MB rotation) and `follows.json` (atomic write).
- [ ] Chat rooms over TLS to `irc.chat.twitch.tv:6697` as a guest, long poll of at most 25 s, 200
      events kept, 12 rooms, 60 s idle; emotes merged and cached as the prototype does.
- [x] Tests against a fake Twitch (`crates/service/tests/fake_twitch.rs`): home session, clean
      stream, pre-roll cut and recorded, mid-roll with a discontinuity, offline channel, a passing
      error retried, a dead exit sending the session home, follows, the page and its headers.
- [ ] Tests still missing ([task 4](tasks/04-fake-twitch.md) grows the fake for them): a live exit
      proxy that goes down and comes back, a dropped connection, the exit's country in status.
- [x] A run against real Twitch from a home line (2026-10-30: the most-watched channel opened from
      Canada with 6 qualities; a 102 s mid-roll was cut, 7 segments, playlist held at 15; no unknown
      titles or classes).
- [ ] A run through an exit, and a player (mpv, the page) on the Rust service for an hour.
- [ ] Behind a flag: pass `#EXT-X-TWITCH-PREFETCH` lines through when not in a break (2 to 4 s less
      delay). Measure before enabling.

### 3. Tools

One binary, `breakless-tools`, with the prototype's five scripts as subcommands: `probe`, `soak`,
`watch-raw`, `capture`, `upstream` ([task 2](tasks/02-upstream-watch.md) for the last). Output
shapes stay as they are so existing scripts that parse them keep working.

### 4. Container and releases ([task 3](tasks/03-container-and-release.md))

Multi-stage image from `rust:1.99.1-alpine` to `scratch`, non-root, a `health` subcommand for the
healthcheck. A release workflow on `v*` tags: static binaries for x86_64 and aarch64 Linux, the
image on GHCR, checksums, the changelog section as release notes.

### 5. Cutover and 1.0

The maintainer runs the Rust service in place of the prototype for a week with the same checks (the
probe through the exit, a 25-minute soak, the page in a browser, a television). Then `1.0.0`, and
`reference/python/` moves to a tag and leaves the tree.

### 6. After 1.0, in order of value

1. A second exit running beside the first, so a dead region does not mean adverts.
2. A backup stream during a break instead of a frozen picture (what vaft calls BackupSwapFirst):
   measure first whether a second token kind is ever clean mid-roll.
3. A filler clip while adverts are cut without an exit, so players that give up on an opening
   pre-roll do not.
4. Recordings and clips: capture a VOD playlist first and read whether it carries adverts.
5. Badge pictures, clickable links and search in chat.

## Staying ahead of Twitch

Twitch changes how adverts are marked a few times a year. The tripwires, in order of how early they
fire:

| Tripwire | When | Goes off when | Then |
|---|---|---|---|
| `breakless-tools upstream` | weekly | vaft or Streamlink look for a mark, address or token kind that `breakless-formats` does not account for, or their client id differs | read their change, decide, change the parser and its tests, record the decision |
| `/api/status` | every look | `segment_titles_not_seen_before` or `daterange_classes_not_seen_before` is not empty | capture the playlist while it is there, read what the segments under the mark are titled, add a fixture and a test |
| `breakless-tools probe` through the exit | monthly | a channel came back with an advert | the country is no longer clean: measure others, move |
| `adverts.jsonl` | monthly | lines with `"from": "exit"` | the same |

What none of these catch: an advert that carries no mark and is titled `live`. Nobody has seen one.
If a person reports an advert shown while the status counts no break, capture the playlist during it
and compare with `fixtures/playlists/home-clean-media.m3u8` line by line.

## YouTube

Asked for on 2026-10-30. Research only so far; the facts below were read that day and are dated
where the source is.

**What is known.**

- YouTube's adverts are mostly chosen and fetched by the player, separately from the video. A player
  that only plays the media streams shows none. That is why Invidious, Piped, NewPipe and players
  built on yt-dlp have no adverts, and why YouTube's effort goes into denying them the streams.
- YouTube tested server-side ad insertion in June 2024 (adverts stitched into the stream, as Twitch
  does). Reports in 2026 say it has spread; none of them is a primary source. Whether a live stream's
  HLS playlist carries stitched adverts today is unmeasured.
- Getting the streams is the hard part. yt-dlp's PO Token Guide (revised 2026-07-12): since
  August 2024 the web clients need a "proof of origin" token for stream requests or get 403; web
  formats are SABR-only (server-controlled, not plain DASH or HLS); **HLS live streams do not need the
  token** except on the iOS client; the `web_safari` client gives HLS formats without a token "at this
  time"; the TV client needs no token but returns DRM'd formats without cookies. Addresses that YouTube
  flags (data-centre and VPN ranges especially) get "Sign in to confirm you're not a bot".
- Public Invidious and Piped instances are being blocked by address; self-hosted ones on a home line
  with few users still work.
- In Rust, [rustypipe](https://codeberg.org/ThetaDev/rustypipe) (0.11, 2026) is an Innertube client
  with a separate helper for the tokens.

**What it means for the design.** For YouTube the two ideas invert. The VPN exit is a liability (a
flagged address), the home line is the asset. The cutting layer is needed only if stitched adverts
turn out to be present. So a YouTube live channel through Breakless is: ask Innertube from the home
line, with the client that hands out plain HLS; rebuild the media playlists through the same
`Timeline` (with YouTube's marks, once seen); the same page and player; live chat read-only through
Innertube's continuation calls. Recordings are out of scope until SABR is understood.

**Phases, gated on measurement.**

- **Y0. Measure** ([task 5](tasks/05-youtube-probe.md)): a probe that fetches twelve live channels'
  HLS masters and media playlists from a home line and reports: which client answered, whether the
  playlist carries anything besides content (`DATERANGE`, discontinuities, odd segment hosts), and
  how often a request met 403 or a bot check. Scrubbed captures become fixtures. Decide after reading
  them whether a token helper is needed at all.
- **Y1. Formats**: Innertube player request and response types for the chosen client, HLS master
  reading (standard tags), YouTube advert marks if Y0 found any.
- **Y2. Service**: `/yt/live/<video-or-channel>.m3u8` beside `/live/`, the channel list's YouTube half,
  status fields per platform.
- **Y3. Chat**: live chat read-only through Innertube.

**Risks to say out loud.** YouTube's terms forbid access by means other than its own clients; the
notice in the README will cover YouTube too. The token helper, if Y0 shows it is needed, runs
YouTube's obfuscated JavaScript in an embedded engine, a heavy and fragile dependency; that is a
decision for the maintainer after Y0, not before. YouTube moves faster than Twitch: expect the
weekly upstream watch to need a yt-dlp and rustypipe row.

## Decisions taken

| Decision | When | Why |
|---|---|---|
| Name: Breakless | 2026-10-30 | No advert breaks. Free on crates.io, eleven unrelated repositories on GitHub, no platform's trademark in it |
| Licence: AGPL-3.0-only | 2026-10-30 | A self-hosted network service; the maintainer's other public project uses it. The bundled hls.js (Apache-2.0) is compatible |
| Hand-written playlist parser, no HLS crate | 2026-10-30 | Twitch's own tags and attributes, and the need to report unknown marks, which a general parser drops |
| Python prototype kept in the tree until 1.0 | 2026-10-30 | It is the specification and the differential oracle |
