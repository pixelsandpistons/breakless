//! `breakless serve` and `breakless health`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use breakless::config::Config;
use breakless::{app, routes};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "breakless",
    version,
    about = "Live Twitch channels with no advert breaks"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Serve playlists, the page and the API.
    Serve(Config),
    /// Exit 0 when a running service answers `/health`. Used as the container's healthcheck.
    Health {
        /// The service's health address.
        #[arg(long, default_value = "http://127.0.0.1:8080/health")]
        url: String,
    },
    /// Print a running service's `/api/status`. For scripts on a host whose container has no shell.
    Status {
        /// The service's status address.
        #[arg(long, default_value = "http://127.0.0.1:8080/api/status")]
        url: String,
    },
}

fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    match Cli::parse().command {
        Command::Serve(config) => serve(config),
        Command::Health { url } => std::process::exit(i32::from(!healthy(&url))),
        Command::Status { url } => match fetch(&url) {
            Some((200, body)) => println!("{body}"),
            Some((code, body)) => {
                eprintln!("HTTP {code}: {}", body.trim());
                std::process::exit(1);
            }
            None => {
                eprintln!("no answer from {url}");
                std::process::exit(1);
            }
        },
    }
}

fn serve(config: Config) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let listen = config.listen;
        let exits = config.proxies();
        let data = config.data.clone();
        let app = Arc::new(app::App::new(config).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        }));
        let reaper = app.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(15));
            loop {
                tick.tick().await;
                reaper.reap();
            }
        });
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .expect("bind the listen address");
        tracing::info!(
            "listening on {listen}; exits {}; data in {}",
            if exits.is_empty() {
                "none (home line, adverts cut)".to_string()
            } else {
                exits.len().to_string()
            },
            data.display()
        );
        axum::serve(listener, routes::router(app))
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
            .expect("serve");
    });
}

/// A plain HTTP/1.0 GET with the standard library: the runtime image has no `curl`. The status
/// code and the body, or `None` when nothing answered.
fn fetch(url: &str) -> Option<(u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (host_port, path) = rest
        .split_once('/')
        .map_or((rest, "/"), |(h, _)| (h, &rest[h.len()..]));
    let mut stream = TcpStream::connect(host_port).ok()?;
    let timeout = Some(Duration::from_secs(5));
    stream.set_read_timeout(timeout).ok()?;
    stream.set_write_timeout(timeout).ok()?;
    let request = format!("GET {path} HTTP/1.0\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n")?;
    let code = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((code, body.to_string()))
}

fn healthy(url: &str) -> bool {
    fetch(url).is_some_and(|(code, _)| code == 200)
}
