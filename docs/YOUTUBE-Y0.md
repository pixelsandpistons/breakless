# YouTube Y0 measurements

Status: built; the live run is the maintainer's. No request to YouTube was made during implementation.
No YouTube service support is built until these measurements have run from a home line and been read.

## Run from the home line

Build with the pinned image, then run the resulting binary on the host. Captures go outside the
repository. `tools/cargo.sh` mounts only the repository, so use the host binary for these captures.

```sh
tools/cargo.sh build --locked -p breakless-tools
./target/debug/breakless-tools youtube-probe --client web_safari --minutes 5 --capture "$HOME/breakless-youtube-y0/web_safari"
./target/debug/breakless-tools youtube-probe --client tv --minutes 5 --capture "$HOME/breakless-youtube-y0/tv"
```

Without video IDs, discovery searches Innertube for `live` using `web_safari`, yt-dlp's video-result
filter and explicit live badges. It follows at most twenty search pages to find twelve live videos
and prints how many it found. Supplying eleven-character video IDs skips discovery. The selected
player client measures all selected videos concurrently, polling each best media playlist every
two seconds for five minutes. `-n` changes the discovery count; `--minutes 0` takes one snapshot.
The HTTP clients ignore proxy environment variables and send no cookies or proof-of-origin tokens.

Capture names are `youtube-<client>-<n>-master.m3u8` and `youtube-<client>-<n>-media.m3u8`, with
one-based row numbers. The media capture starts with the first successful playlist and is replaced
when another discontinuity, class or host is observed. A valid master is still saved if media
requests fail. The tool writes only when `--capture` is given, into that folder.

## Record each run

For each client, replace its “Not run” row with the run date and timezone, whether the line was a
home connection, the command used, the discovery count and the client name. Paste the complete
table, summary line and per-video details below it. Retain `LOGIN_REQUIRED`, bot-check flags and
errors, including runs with no HLS. Copy every distinct DATERANGE class and segment host into the
observation table. Record the capture folder and file names.

| Client | Date and timezone | Line: home or other | Videos selected | Run status |
|---|---|---|---|---|
| web_safari | Not run | Not measured | Not measured | Not run |
| tv | Not run | Not measured | Not measured | Not run |

### web_safari output

Not run. Paste the table, summary line and per-video details after the home-line measurement.

### tv output

Not run. Paste the table, summary line and per-video details after the home-line measurement.

| Client and video row | DATERANGE classes | Segment hosts | Discontinuities | HTTP errors and playability status | Capture files |
|---|---|---|---|---|---|
| Not measured | Not measured | Not measured | Not measured | Not measured | Not measured |

## Read the results

- **Answered** counts valid JSON player responses, including rejected playability statuses.
- **With HLS** counts responses containing `hlsManifestUrl`, even if fetching the manifest fails.
- **With anything besides content** counts rows with a discontinuity, any DATERANGE class, or more
  than one segment host. These are observations to read, not classifications of adverts. All
  segment hosts are printed so a single unexpected host can also be reviewed.
- **Errors** counts failed HTTP requests, malformed responses, rejected playability statuses and
  capture failures. Repeated polling failures count separately; detail lines group equal errors.
  A missing HLS URL with playability status OK is an observation, not an error.
- Segment and discontinuity totals count distinct observations across overlapping windows.
  The tool does not fetch segment bodies or decrypt content.
- Exit codes: 0 means no recorded errors; 1 means measurement completed with recorded errors;
  2 means setup or discovery prevented measurement.

After reading both runs, record whether either client returned usable HLS, whether requests met
403 or a bot check, what the classes and host changes mean after inspecting the scrubbed captures,
and whether another measurement is needed. Decide whether a token helper or cutting layer is
needed only after that review.

Review decision: not made; measurements have not run.

## Client source

Client identities were read on 2026-10-31 from yt-dlp commit
[`b77c9304e1c4220cca450c583491904daf8455af`](https://github.com/yt-dlp/yt-dlp/commit/b77c9304e1c4220cca450c583491904daf8455af).
The version, user agent, numeric client-name header and iOS device fields come from
[`INNERTUBE_CLIENTS`](https://github.com/yt-dlp/yt-dlp/blob/b77c9304e1c4220cca450c583491904daf8455af/yt_dlp/extractor/youtube/_base.py).
The request also includes yt-dlp's static HTML5 playback preference and its English/UTC context.

That source says `web_safari` returns HLS only for some logged-in or trusted sessions. The
[PO Token Guide](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide) says the TV client returns DRM
formats without cookies; the client source says iOS live HLS requires a PO token after thirty
seconds. These restrictions are reasons to measure, not to add cookies or a token helper to Y0.

Discovery's video-result parameter is from
[`_search.py`](https://github.com/yt-dlp/yt-dlp/blob/b77c9304e1c4220cca450c583491904daf8455af/yt_dlp/extractor/youtube/_search.py),
and live badges and continuations follow
[`_tab.py`](https://github.com/yt-dlp/yt-dlp/blob/b77c9304e1c4220cca450c583491904daf8455af/yt_dlp/extractor/youtube/_tab.py).
The client definitions and fixed request tests live together in
[`client.rs`](../crates/tools/src/youtube_probe/client.rs) so they can be refreshed together.
