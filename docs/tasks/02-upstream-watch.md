# Task 02: the upstream watcher as `breakless-tools upstream`

Status: open.

## Goal

Port `reference/python/upstream.py` to Rust as the `upstream` subcommand of a new
`crates/tools` binary (`breakless-tools`), keeping its output lines and exit code so the weekly
routine in `docs/PLAN.md` does not change.

## Read first

- `reference/python/upstream.py`, whole file: `SOURCES`, `ACCOUNTED`, `js_list`, `facts`, `compare`,
  `main`, and the state file's shape (`upstream-state.json`: `facts`, `files` digests, `looked`).
- `reference/python/test_twitchlib.py`, class `Upstream`, for what the tests check.
- `crates/formats/src/playlist.rs`: `CLIENT_ID`, `PLAYER_TYPES`, `AD_URLS`, `KNOWN_CLASSES`, which
  `compare` reads.

## Behaviour to keep

- Downloads the five sources into a folder (`--dir`, default `upstream/`, which `.gitignore` lists)
  with a 30 s timeout, refuses a body under 500 bytes, and on failure uses the copy already there.
- `--offline` compares the copies without downloading and does not write the state file.
- Prints the one-line summary, `changed since the last look: ...` per file whose digest changed,
  `was:`/`now:` pairs for facts that changed, then `NEW` and `GONE` lines, then either
  `nothing new: ...` or nothing; exit 1 when any `NEW` or `GONE` line was printed.
- `ACCOUNTED` moves to Rust as a `&[(&str, &str)]` with the same keys and the same explanations.

## Dependencies

`reqwest` (blocking feature is acceptable here) with `rustls-tls`, `regex`, `serde_json`, `sha2`,
`clap` with `derive`. All MIT or Apache-2.0. Pin them; note each in the pull request.

## Accepted when

- `tools/cargo.sh test -p breakless-tools` passes tests that feed small synthetic snippets (a
  `scope.AdSignifiers = [...]` block, a `def _is_daterange_ad` body, a `CLIENT_ID = "..."` line) and
  check `facts` and `compare`, including: a new mark is reported, an accounted one is not, a different
  client id is reported, a missing block gives `GONE`.
- Running `breakless-tools upstream --offline --dir <folder with the five real files>` on the
  maintainer's machine prints `nothing new` for the sources as of 2026-10-30 (vaft 68.5.7). The
  maintainer runs this; say in the pull request that it was not run by you if it was not.
- No upstream source file is added to the repository.
