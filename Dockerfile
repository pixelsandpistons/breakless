# The service in one static binary on an empty image. Build: docker build -t breakless:dev .
#
# The build stage is the official Alpine image for the pinned toolchain (the tag is the version
# rust-toolchain.toml names). Alpine's Rust targets musl, so the binaries need no libc at run time.
# gcc and the musl headers are for `ring` (TLS); nothing else has C in it.
FROM rust:1.99.1-alpine AS build
RUN apk add --no-cache gcc musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p breakless -p breakless-tools

# Nothing but the two binaries, the page and the trust roots. No shell: the healthcheck is the
# binary's own `health` subcommand, and `breakless status` prints /api/status for scripts.
FROM scratch
COPY --from=build /src/target/release/breakless /breakless
COPY --from=build /src/target/release/breakless-tools /breakless-tools
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY web /web
USER 65532:65532
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=5 CMD ["/breakless", "health"]
ENTRYPOINT ["/breakless"]
CMD ["serve"]
