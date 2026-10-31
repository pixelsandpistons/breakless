# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow
[Semantic Versioning](https://semver.org/) once there is a release.

## Unreleased

### Added

- `breakless-formats`: master and media playlist parsing, advert marks, the rebuilt `Timeline`,
  attribute-list and date parsing. 25 tests, including a byte-for-byte comparison with the Python
  prototype's output on the captured playlists under `fixtures/`.
- `breakless` (the service): sessions and exits with the prototype's fallback rules, the rebuilt
  playlists, `/api/status` (plus an `exits` list and the proxy's host per exit), `/api/watch`,
  `/api/channels`, `/api/follow`, the page, `adverts.jsonl` and `follows.json`. Ten tests drive it
  against a fake Twitch on loopback. `/api/chat` and `/api/emotes` answer 501 until the chat lands.
- `breakless-formats`: chat lines and tags, the four event kinds, the room's rules, the numbered
  log of the last 200 events, and the three emote providers' list readers and merge. 17 tests;
  two compare the JSON with the prototype's for the same message and the same six lists.
- `breakless` (the service): `/api/chat` (guest rooms over TLS, the 25 s long poll, 12 rooms,
  left after 60 s idle) and `/api/emotes` (the lists fetched together and kept: global an hour,
  a channel's ten minutes, a failure a minute), and the `chat` list in `/api/status`. Four tests
  against a fake chat server and fake providers.
- `breakless status`: prints a running service's `/api/status`, for scripts on a host whose
  container has no shell.
- `breakless-tools probe`: pre-roll measurement per token kind, straight out or through a proxy,
  with the prototype's table and summary lines.
- `breakless-tools soak`: watches channels through a running service and reports breaks, stalls
  and errors per channel, as the prototype's `soak.py` does.
- `breakless-tools watch-raw` and `capture`: follow a channel's playlist as Twitch sends it, and
  save a scrubbed master and media pair as a fixture (address, tokens, long paths, session ids,
  channel name).
- `breakless-tools upstream`: fetches the five watched sources, falls back to saved copies, and
  preserves the prototype’s reports, exit codes and `upstream-state.json`. `--offline` compares
  without downloading or writing state; the folder comes from `--dir`, `BREAKLESS_UPSTREAM_DIR`,
  or `upstream` under the current working directory.
- `fake-twitch`: an unpublished test crate holding the service's loopback Twitch, IRC and emote
  fakes. Adds timed channel windows, advert-break controls, dropped connections, per-channel
  statuses and countries, request records with `Via`, and an HTTP exit proxy with down/up
  controls. The service uses it only as a dev-dependency.
- Four service tests against the fake: live exit routing and country, fallback when an active
  exit goes down, a dropped media request retried in the same session, and a dropped token request
  retried through the exit before the session opens.
- `breakless-tools youtube-probe`: measures live HLS access for four Innertube clients, counts
  playlist observations and HTTP failures, and saves scrubbed captures only with `--capture`.
  Offline tests cover the client requests, counting and scrubbing; the home-line run is the
  maintainer's. The run template is `docs/YOUTUBE-Y0.md`.
- The Python prototype as `reference/python/`, the page as `web/`, a generic compose file as
  `deploy/`, the pinned build image under `tools/build-image/` and the `tools/cargo.sh` wrapper.
- `Dockerfile`: both binaries built statically on the pinned Alpine image and shipped on `scratch`
  with the page and the trust roots, as user 65532, with `breakless health` as the healthcheck.
  `deploy/docker-compose.image.yml` runs that image; `.github/workflows/release.yml` builds the
  binaries for x86_64 and aarch64 Linux on a `v*` tag, pushes the image to GHCR and makes the
  release with the checksums and the changelog section.
- The plan (`docs/PLAN.md`), the architecture notes and the first contributor tasks.

### Changed, relative to the prototype as it ran before this repository

- The capture tool also scrubs `SERVING-ID`, `VIDEO-SESSION-ID` and `BROADCAST-ID`.
- The soak and page tests default to `http://127.0.0.1:8080` instead of the maintainer's hostname.
- The prototype's compiled bytecode (`__pycache__/`) is no longer in the repository.
- Attribute lookup in `DATERANGE` and `STREAM-INF` lines matches whole keys only, so `DURATION`
  cannot match inside `X-TV-TWITCH-AD-POD-FILLED-DURATION`. The prototype matched the first
  occurrence; on every captured playlist the result is the same.
