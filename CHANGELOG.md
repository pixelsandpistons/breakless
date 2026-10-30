# Changelog

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow
[Semantic Versioning](https://semver.org/) once there is a release.

## Unreleased

### Added

- `breakless-formats`: master and media playlist parsing, advert marks, the rebuilt `Timeline`,
  attribute-list and date parsing. 25 tests, including a byte-for-byte comparison with the Python
  prototype's output on the captured playlists under `fixtures/`.
- The Python prototype as `reference/python/`, the page as `web/`, a generic compose file as
  `deploy/`, the pinned build image under `tools/build-image/` and the `tools/cargo.sh` wrapper.
- The plan (`docs/PLAN.md`), the architecture notes and the first contributor tasks.

### Changed, relative to the prototype as it ran before this repository

- The capture tool also scrubs `SERVING-ID`, `VIDEO-SESSION-ID` and `BROADCAST-ID`.
- The soak and page tests default to `http://127.0.0.1:8080` instead of the maintainer's hostname.
- Attribute lookup in `DATERANGE` and `STREAM-INF` lines matches whole keys only, so `DURATION`
  cannot match inside `X-TV-TWITCH-AD-POD-FILLED-DURATION`. The prototype matched the first
  occurrence; on every captured playlist the result is the same.
