# Contributing

Thank you for looking. This page covers the mechanics; the plan and the open tasks are in
[docs/PLAN.md](docs/PLAN.md) and [docs/tasks/](docs/tasks/).

## Set up

Docker is enough:

```sh
docker build -t breakless-build:1.99.1 tools/build-image
tools/cargo.sh test --locked
```

Or a local toolchain: `rust-toolchain.toml` pins the version and adds `rustfmt` and `clippy`.

## Before a pull request

All of these must pass; CI runs the same:

```sh
tools/cargo.sh fmt --all --check
tools/cargo.sh clippy --all-targets --locked -- -D warnings
tools/cargo.sh test --locked
RUSTDOCFLAGS="-D warnings" tools/cargo.sh doc --no-deps --locked
python3 reference/python/test_twitchlib.py && python3 reference/python/test_chat.py
python3 fixtures/gen-expected.py && git diff --exit-code fixtures/expected
```

The last line keeps the Python prototype and the Rust code in step: the Rust tests compare their
output with what the prototype produces for the same fixtures, byte for byte.

## Rules of the road

- **One topic per pull request.** Say what was tested and how; say "fixture" when it was a fixture
  and "live" when it was against Twitch.
- **A behaviour change comes with a test.** A change in what Twitch sends comes with a scrubbed
  capture under `fixtures/playlists/` (`reference/python/capture.py` scrubs the viewer's address,
  tokens, segment paths and session ids; never commit an unscrubbed playlist) and its expected output
  regenerated.
- **Preserve the contract.** Every route and JSON field of the prototype (`reference/python/server.py`,
  top of the file) stays as it is unless the change log says otherwise. The rebuilt playlist's
  semantics are documented in `crates/formats/src/timeline.rs`.
- **Rust for application logic.** The page is HTML, CSS and plain browser glue and stays that way.
  No new Python beyond fixing the prototype while it still runs.
- **Dependencies are pinned.** `Cargo.lock` is committed; `deny.toml` lists the licences allowed.
  Prefer the standard library; add a crate only for something it cannot do well.
- **Write plainly.** A fact lives in one place and the rest links to it. No placeholders, no dates in
  code comments (they go in `CHANGELOG.md`), no hedged claims in the README: a statement is either
  tested or listed under what the project does not do. Comments say why, not what.
- **Nothing private.** No hostnames, addresses, tokens or captured playlists that identify a viewer.

## Made with AI

Most of this repository is written by an AI coding agent directed by the maintainer. Contributions
written with AI help are welcome on the same terms as any other: you read it, you ran the checks,
you stand behind it. Say in the pull request what was generated and what you verified yourself.
