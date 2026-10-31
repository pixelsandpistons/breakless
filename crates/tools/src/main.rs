//! `breakless-tools`: the measuring tools beside the service.
//!
//! `probe`, `soak`, `watch-raw`, `capture` and `upstream` are ports of the prototype's scripts.

mod capture;
mod probe;
mod soak;
mod upstream;
mod watch_raw;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "breakless-tools",
    version,
    about = "Measuring tools for Breakless"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Measure pre-roll adverts right now: which channels open clean, from where.
    Probe(probe::Args),
    /// Watch channels through a running service for a while and report breaks and stalls.
    Soak(soak::Args),
    /// Follow one channel's playlist as Twitch sends it and print every change of tags and titles.
    WatchRaw(watch_raw::Args),
    /// Save a scrubbed master and media playlist pair as a fixture.
    Capture(capture::Args),
    /// Compare upstream projects' advert marks, client ids and token kinds with ours.
    ///
    /// The folder comes from `--dir`, then `BREAKLESS_UPSTREAM_DIR`, then `upstream` relative to the
    /// current working directory.
    Upstream(upstream::Args),
}

fn main() {
    breakless::twitch::install_tls();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let code = runtime.block_on(async {
        match Cli::parse().command {
            Command::Probe(args) => probe::run(args).await,
            Command::Soak(args) => soak::run(args).await,
            Command::WatchRaw(args) => watch_raw::run(args).await,
            Command::Capture(args) => capture::run(args).await,
            Command::Upstream(args) => upstream::run(args).await,
        }
    });
    std::process::exit(code);
}
