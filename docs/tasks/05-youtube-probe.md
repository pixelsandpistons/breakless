# Task 05: the YouTube live measurement probe

Status: open. This is phase Y0 of [the plan](../PLAN.md#youtube). Nothing about YouTube is built
until this has run and been read.

## Goal

A `youtube-probe` subcommand of `breakless-tools` that, from a home line, finds live channels, asks
Innertube for their HLS playlists with the clients that are said to hand out plain HLS without a
proof-of-origin token, and reports what came back. Its output decides whether YouTube support is
feasible without a token helper and whether a cutting layer is needed at all.

## Read first

- yt-dlp's PO Token Guide (github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide), the client table: which
  clients need a token for what; the note that HLS live streams do not need one except on iOS; the
  `web_safari` and `tv` rows. Also the Innertube `player` request body each client sends (`context.client`
  with `clientName`, `clientVersion`, and for `web_safari` the Safari user agent); yt-dlp's
  `yt_dlp/extractor/youtube/_base.py` lists them.
- `crates/formats/src/playlist.rs` for what the Twitch probe records per channel, and
  `reference/python/probe.py` for the table it prints.

## Deliver

1. The subcommand: `breakless-tools youtube-probe [--client web_safari|tv|ios|mweb] [-n 12] [--minutes 5] [IDS...]`.
   Without ids: twelve live videos from a search for `live` or the Live browse page (say which).
   For each: the `player` response's `playabilityStatus`, whether `hlsManifestUrl` was present, the
   HLS master's variants, then the media playlist of the best variant polled every 2 s for
   `--minutes`: count segments, `#EXT-X-DISCONTINUITY`, every distinct `#EXT-X-DATERANGE` class and
   every distinct segment host. Record HTTP errors and any `LOGIN_REQUIRED` or bot-check status.
2. A table at the end, one row per channel, and a summary line per client: answered / with HLS /
   with anything besides content / errors.
3. `--capture DIR`: writes each master and one media playlist, with the viewer's address, the
   signatures (`sig`, `signature`, `ip`, `expire`, `id` query parameters and any token-like value over
   40 characters) replaced, and the channel renamed, ready for `fixtures/playlists/youtube-*.m3u8`.
4. A `docs/YOUTUBE-Y0.md` template the person running it fills in: date, line (home or not), client,
   the summary lines, and what the `DATERANGE` classes and segment hosts were.

## Dependencies

`reqwest`, `serde_json`, `clap` as in task 02. No JavaScript engine, no token helper, no cookies.

## Accepted when

- The code builds and its unit tests cover the request bodies per client and the counting over a
  synthetic media playlist with a discontinuity and a `DATERANGE`.
- The maintainer runs it from a home line for `web_safari` and one other client and fills in
  `docs/YOUTUBE-Y0.md`. Say in the pull request that the live run is the maintainer's.
- Nothing is written to the repository by the tool except through `--capture` into a folder the
  person chooses.
