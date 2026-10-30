# Build image: the official Alpine Rust image for the pinned toolchain plus the lint components.
# Nothing else is installed; the crates have no C dependencies.
FROM rust:1.99.1-alpine
RUN rustup component add rustfmt clippy
