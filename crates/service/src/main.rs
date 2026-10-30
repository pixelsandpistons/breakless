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
}

fn main() {
    tracing_subscriber::fmt().with_target(false).init();
    match Cli::parse().command {
        Command::Serve(config) => serve(config),
        Command::Health { url } => std::process::exit(i32::from(!healthy(&url))),
    }
}

fn serve(config: Config) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let listen = config.listen;
        let exits = config.proxies();
        let data = config.data.clone();
        let app = Arc::new(app::App::new(config).expect("HTTP clients"));
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

/// A plain HTTP/1.0 GET with the standard library: the runtime image has no `curl`.
fn healthy(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let (host_port, path) = rest
        .split_once('/')
        .map_or((rest, "/"), |(h, _)| (h, &rest[h.len()..]));
    let Ok(mut stream) = TcpStream::connect(host_port) else {
        return false;
    };
    let timeout = Some(Duration::from_secs(5));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    let request = format!("GET {path} HTTP/1.0\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 64];
    let Ok(n) = stream.read(&mut head) else {
        return false;
    };
    String::from_utf8_lossy(&head[..n]).starts_with("HTTP/1.")
        && head[..n].windows(4).any(|w| w == b" 200")
}
