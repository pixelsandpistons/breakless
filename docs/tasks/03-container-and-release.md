# Task 03: the container image and the release workflow

Status: done except the last acceptance line: the `Dockerfile`, `.dockerignore`,
`deploy/docker-compose.image.yml` and `.github/workflows/release.yml` are in the tree and the image
checks below passed on x86_64; the workflow has not yet run on a tag. The brief stays as the record.

## Goal

A `Dockerfile` that builds the `breakless` service binary statically and ships it on `scratch`, and a
GitHub Actions workflow that, on a `v*` tag, builds the binaries, publishes the image to GHCR and
makes a GitHub release.

## Read first

- `crates/service/` as it stands: a `clap` binary named `breakless` with `serve` and `health`
  subcommands. `health` exits 0 when the service answers; the healthcheck uses it because `scratch`
  has no `curl`.
- `deploy/docker-compose.yml`: the prototype's compose file, whose `breakless` service this image
  replaces (`image:` instead of `python:3.12-alpine` and the two mounts).
- `.github/workflows/ci.yml` for the toolchain and caching conventions.
- `tools/build-image/Dockerfile` for the pinned builder.

## Deliver

1. `Dockerfile` at the repository root:
   - build stage `FROM rust:1.99.1-alpine`, `cargo build --release --locked -p breakless`, target the
     image's native musl so the binary is static;
   - runtime `FROM scratch`: the binary, `web/` at `/web`, CA certificates copied from the builder,
     `USER 65532:65532`, `EXPOSE 8080`, `HEALTHCHECK CMD ["/breakless", "health"]`,
     `ENTRYPOINT ["/breakless"]`, `CMD ["serve"]`;
   - `.dockerignore` excluding `target/`, `reference/`, `fixtures/`, `docs/`.
2. `.github/workflows/release.yml`: on tags `v*`: build `x86_64-unknown-linux-musl` and
   `aarch64-unknown-linux-musl` (cross-compiling the second with `cross` or a zig linker; say which
   and why), `sha256sum` them, build and push the image for both architectures to
   `ghcr.io/<owner>/breakless` with the tag and `latest`, and create the release with the binaries, the
   checksums and the matching section of `CHANGELOG.md` as the body.
3. A `deploy/docker-compose.image.yml` variant that uses the GHCR image and keeps the hardening
   (read-only, non-root, no capabilities, tmpfs).

## Accepted when

- `docker build -t breakless:dev .` succeeds on x86_64 and `docker run --rm breakless:dev health --url http://127.0.0.1:1`
  exits non-zero quickly (nothing listening), while `docker run --rm -p 127.0.0.1:8080:8080 breakless:dev`
  answers `curl http://127.0.0.1:8080/health` with 200.
- The image is under 20 MB.
- The workflow passes `actionlint` and the maintainer's dry run on a `v0.0.1-test` tag in a fork.
- No secrets in the workflow beyond `GITHUB_TOKEN`.
