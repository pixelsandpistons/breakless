//! `breakless-tools`: the measuring tools beside the service.
//!
//! Today: `probe`. The prototype's `soak.py`, `watch_raw.py`, `capture.py` and `upstream.py` are
//! the specifications for the subcommands still to come (`docs/PLAN.md`, phase 3).

mod probe;

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
}

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let code = runtime.block_on(async {
        match Cli::parse().command {
            Command::Probe(args) => probe::run(args).await,
        }
    });
    std::process::exit(code);
}
