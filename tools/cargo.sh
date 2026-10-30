#!/bin/sh
# Runs cargo inside the pinned build image, so nothing needs installing on the host.
#   tools/cargo.sh test --locked
#   tools/cargo.sh clippy --all-targets -- -D warnings
# Build the image once: docker build -t breakless-build:1.99.1 tools/build-image
cd "$(dirname "$0")/.." || exit 1
mkdir -p "${BREAKLESS_CARGO_CACHE:-$HOME/.cache/breakless-cargo}"
exec docker run --rm -v "$PWD":/w -w /w \
  -v "${BREAKLESS_CARGO_CACHE:-$HOME/.cache/breakless-cargo}":/usr/local/cargo/registry \
  -e CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-always}" breakless-build:1.99.1 cargo "$@"
