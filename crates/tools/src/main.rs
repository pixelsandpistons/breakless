//! `breakless-tools`: the measuring tools beside the service.
//!
//! Today: `probe` and `soak`. The prototype's `watch_raw.py`, `capture.py` and `upstream.py` are
//! the specifications for the subcommands still to come (`docs/PLAN.md`, phase 3).

mod probe;
mod soak;

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
}

fn main() {
    breakless::twitch::install_tls();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let code = runtime.block_on(async {
        match Cli::parse().command {
            Command::Probe(args) => probe::run(args).await,
            Command::Soak(args) => soak::run(args).await,
        }
    });
    std::process::exit(code);
}
