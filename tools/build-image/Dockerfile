# Build image: the official Alpine Rust image for the pinned toolchain plus the lint components.
# gcc and the musl headers are for `ring` (TLS); nothing else has C in it.
FROM rust:1.99.1-alpine
RUN apk add --no-cache gcc musl-dev && rustup component add rustfmt clippy
