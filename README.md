# Breakless

Live Twitch channels with no advert breaks, handed to any player you already have as plain HLS
playlists. Self-hosted, written in Rust, made with AI.

> **Early development.** The Rust service in [`crates/`](crates/) is what runs at the maintainer's home
> since 2026-10-31, in place of the Python prototype that ran there from 4 October 2026 and is kept in
> [`reference/python/`](reference/python/) as the specification until 1.0. Every route of the prototype
> is served; see [Status](#status) for what has been measured and what has not.

## What it does

Twitch stitches adverts into the video playlist on its own servers, separately for each viewer, and
decides whether to by where the playlist request appears to come from. No list of blocked names can
touch them. Breakless does two things:

1. **Asks from somewhere Twitch sells no adverts.** The playlists are requested through an HTTP proxy
   in such a country: a [gluetun](https://github.com/qdm12/gluetun) container with whichever VPN
   provider you already pay for. Only the playlists cross that line, a few kilobytes a second.
2. **Cuts what still arrives.** Any advert segment that reaches the service is dropped and the stream
   is renumbered so players do not trip. The picture pauses for the length of the break instead of
   showing it. This is also the whole behaviour when no proxy is configured.

The video files themselves go from Twitch straight to the player. The service never carries video.

```
browser / mpv / VLC / TV ──▶ breakless ──▶ gluetun (HTTP proxy, advert-free country) ──▶ Twitch: token, playlists
          │                      │  no proxy, or the proxy fails: straight out, adverts cut
          ▼
   Twitch's video servers ◀── the player fetches the 2-second video files itself
```

Give `http://<host>:8080/live/CHANNEL.m3u8` to mpv, VLC, Kodi or a television, or open the built-in
page: a channel list, a player, read-only chat with BetterTTV, FrankerFaceZ and 7TV emotes.

## Status

| Piece | State |
|---|---|
| Python prototype (`reference/python/`) | Running at the maintainer's home since 2026-10-04 through an exit in Iceland: 0 advert breaks seen through the exit in every soak and status read since. 41 offline tests. Standard library only |
| `breakless-formats` (Rust) | Playlist parsing, advert marks, the rebuilt timeline: 25 tests, including a byte-for-byte comparison with the prototype's output on real captured playlists |
| `breakless` service (Rust) | Playlists, sessions, exits with fallback, status, the channel list, follows, the page, chat, emotes: built, 18 tests against a fake Twitch in `crates/service/tests/`. Same routes and fields as the prototype. Run against real Twitch from a home line on 2026-10-30: a real mid-roll of 102 s was cut (7 segments) with no unknown marks. Through an exit in Iceland for an hour on 2026-10-31: the page in headless Chromium played the most-watched channel at 1080p for 3,600 s with no console errors while a soak watched it and four more channels: 9,074 segments passed, 0 cut, 0 s stalled, 0 errors, no unknown marks. Since 2026-10-31 05:52 UTC it is the service running at the maintainer's home |
| Chat and emotes (Rust) | Built: guest chat rooms over TLS with the prototype's long poll, the three emote providers merged and cached. The prototype's 18 offline tests are ported (12 on the parsing in `breakless-formats`, 4 against a fake chat server and fake providers in the service); two compare the JSON with what `chat.py` produces, field for field. Run once against real Twitch from a home line on 2026-10-30: joined the most-watched channel's chat, messages arrived with their emote positions, 131 emote words from the three providers with no errors |
| Tools (Rust, `breakless-tools`) | `probe` built; same table and last lines as the prototype's, run once from a home line (3 of 3 pre-rolls, as expected there). `soak`, `watch-raw` and `capture` built (`capture`'s scrub is tested against the fixtures; `soak` and `watch-raw` drive a running service and have no automated test). `upstream` built with 19 offline tests; on the five real sources its output, exit code and state file matched the prototype's byte for byte on 2026-10-31 ([task 02](docs/tasks/02-upstream-watch.md)). `youtube-probe` built with offline tests for client requests, playlist counting and capture scrubbing; the live run is the maintainer's ([Y0 measurements](docs/YOUTUBE-Y0.md)) |
| Container image, releases | `Dockerfile` builds both binaries statically and ships them on `scratch` with the page and the trust roots: 17.6 MB, non-root, the healthcheck is `breakless health`. Built and checked on x86_64 on 2026-10-30. The release workflow (`v*` tags: binaries for x86_64 and aarch64, the image on GHCR, checksums, the changelog section as notes) is written and not yet run on a tag |
| YouTube | Research only; see [the plan](docs/PLAN.md#youtube) |

The plan, phase by phase, is [docs/PLAN.md](docs/PLAN.md). Tasks that are ready for a contributor
(human or AI) are in [docs/tasks/](docs/tasks/).

## Run it

Needs Docker. Copy [`deploy/.env.example`](deploy/.env.example) to `deploy/.env`, fill in your VPN
provider's settings as gluetun names them, pick a country, then:

```sh
cd deploy
docker compose up -d
curl -s http://127.0.0.1:8080/api/status
mpv http://127.0.0.1:8080/live/CHANNEL.m3u8
```

That compose file runs the Python prototype from the source tree. `deploy/docker-compose.image.yml`
is the same with the Rust service from `ghcr.io/pixelsandpistons/breakless` instead; no image is
published there until the first release, so until then build one yourself:

```sh
docker build -t breakless:dev .
```

and put `breakless:dev` in place of the `ghcr.io` name.

`/api/status` says which country Twitch placed the request in (`exit.country`) and counts every
advert break met and cut. Leave `TWITCH_PROXY` empty to run without a VPN: adverts are then cut and the
picture waits them out.

Countries where Twitch currently sells no adverts change over time. Iceland was measured clean on
12 of 12 channels on 2026-10-04 and again on 2026-10-30; Portugal was clean in daily checks on
2026-10-10. Measure before trusting a country: `docker exec breakless python /app/probe.py -n 12 --proxy http://exit:8888`.

This compose file is the maintainer's setup made generic. It has not yet been started from scratch
on another machine; if it does not come up for you, open an issue with the output of `docker compose logs`.

## Build and check the Rust code

Nothing to install but Docker:

```sh
docker build -t breakless-build:1.99.1 tools/build-image
tools/cargo.sh fmt --all --check
tools/cargo.sh clippy --all-targets --locked -- -D warnings
tools/cargo.sh test --locked
```

With a local toolchain, `cargo test --locked` does the same; `rust-toolchain.toml` pins the version.
[CONTRIBUTING.md](CONTRIBUTING.md) has the rest.

## What it does not do

- **twitch.tv itself, the Twitch apps on phones, consoles and televisions.** They talk to Twitch
  directly. For twitch.tv in a browser, the [TTV LOL PRO](https://github.com/younesaassila/ttv-lol-pro)
  extension does the same trick inside the site and can be pointed at your own proxy.
- **Recordings and clips.** Live channels only, for now.
- **Signing in, writing in chat, channel points, drops.** The service has no Twitch account. Chat is
  read as a guest.
- **Authentication of its own.** Put it behind your LAN, your VPN or your own reverse proxy with
  sign-in. See [SECURITY.md](SECURITY.md).

## Please read this before using it

Breakless talks to Twitch's unofficial GraphQL endpoint with the public web player's client id, which
Twitch does not support for third parties, and it exists to avoid adverts that pay the people you
watch. Twitch Turbo removes adverts everywhere, including the apps; a subscription to a channel
removes them on that channel and pays its streamer. Those are the supported ways. Use Breakless at
your own risk and within the law and the terms that apply to you.

Breakless is not affiliated with or endorsed by Twitch or Amazon.

## Made with AI

The code, tests and documentation in this repository are written by an AI coding agent (Claude Code),
directed by the maintainer. Every change is meant to pass the checks above before it lands, but treat
the code as you would any early, unreviewed project. Contributions from people are welcome.

## Licence

[GNU AGPL-3.0-only](LICENSE). The page bundles hls.js under Apache-2.0; see
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
