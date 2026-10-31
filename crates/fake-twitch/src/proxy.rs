//! An HTTP proxy confined to the fake's loopback endpoints.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use hyper::Request;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::task::{JoinHandle, JoinSet};

struct Control {
    down: AtomicBool,
    changed: broadcast::Sender<()>,
}

struct Server(JoinHandle<()>);

impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A plain HTTP proxy that forwards only to this fake's loopback listeners.
#[derive(Clone)]
pub struct Proxy {
    /// Address to configure as the service's HTTP proxy.
    pub addr: SocketAddr,
    control: Arc<Control>,
    _server: Arc<Server>,
}

impl Proxy {
    /// The address in HTTP proxy URL form.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Closes current connections and refuses subsequent requests until `up` is called.
    pub fn down(&self) {
        self.control.down.store(true, Ordering::SeqCst);
        let _ = self.control.changed.send(());
    }

    /// Allows requests through the same listening address again.
    pub fn up(&self) {
        self.control.down.store(false, Ordering::SeqCst);
    }
}

/// Starts a proxy whose only forwarding targets are the given fake listeners.
pub(crate) async fn start(allowed: [SocketAddr; 3]) -> io::Result<Proxy> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (changed, _) = broadcast::channel(16);
    let control = Arc::new(Control {
        down: AtomicBool::new(false),
        changed,
    });
    let server_control = Arc::clone(&control);
    let server = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((socket, _)) = accepted else { break };
                    let control = Arc::clone(&server_control);
                    let mut changed = control.changed.subscribe();
                    if control.down.load(Ordering::SeqCst) { continue; }
                    connections.spawn(async move {
                        let service_control = Arc::clone(&control);
                        let service = service_fn(move |request| {
                            forward(request, Arc::clone(&service_control), allowed)
                        });
                        let connection = hyper::server::conn::http1::Builder::new();
                        tokio::select! {
                            _ = connection.serve_connection(TokioIo::new(socket), service) => {}
                            _ = changed.recv() => {}
                        }
                    });
                }
                _ = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    });
    Ok(Proxy {
        addr,
        control,
        _server: Arc::new(Server(server)),
    })
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let named = headers
        .get("connection")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(',')
        .map(|name| name.trim().to_string())
        .collect::<Vec<_>>();
    for name in named {
        headers.remove(name.as_str());
    }
    for name in [
        "connection",
        "proxy-connection",
        "proxy-authenticate",
        "proxy-authorization",
        "keep-alive",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

async fn forward(
    request: Request<Incoming>,
    control: Arc<Control>,
    allowed: [SocketAddr; 3],
) -> io::Result<Response> {
    if control.down.load(Ordering::SeqCst) {
        return Err(io::Error::other("fake proxy is down"));
    }
    let target = request
        .uri()
        .authority()
        .and_then(|a| a.as_str().parse::<SocketAddr>().ok());
    let Some(target) = target.filter(|target| allowed.contains(target)) else {
        return Ok((StatusCode::BAD_REQUEST, "target is not a fake endpoint").into_response());
    };
    if request.uri().scheme_str() != Some("http") {
        return Ok((StatusCode::BAD_REQUEST, "only plain HTTP is supported").into_response());
    }
    let (mut parts, body) = request.into_parts();
    let path = parts.uri.path_and_query().map_or("/", |path| path.as_str());
    parts.uri = path.parse::<Uri>().map_err(io::Error::other)?;
    strip_hop_headers(&mut parts.headers);
    let previous = parts
        .headers
        .get("via")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let via = if previous.is_empty() {
        "1.1 fake-twitch".to_string()
    } else {
        format!("{previous}, 1.1 fake-twitch")
    };
    parts.headers.insert(
        "via",
        HeaderValue::from_str(&via).map_err(io::Error::other)?,
    );
    parts
        .headers
        .insert("connection", HeaderValue::from_static("close"));
    let body = to_bytes(Body::new(body), usize::MAX)
        .await
        .map_err(io::Error::other)?;
    let socket = TcpStream::connect(target).await?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(socket))
        .await
        .map_err(io::Error::other)?;
    let driver = Server(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let response = sender
        .send_request(Request::from_parts(parts, Body::from(body)))
        .await
        .map_err(io::Error::other)?;
    let (mut parts, body) = response.into_parts();
    let body = to_bytes(Body::new(body), usize::MAX)
        .await
        .map_err(io::Error::other)?;
    strip_hop_headers(&mut parts.headers);
    drop(driver);
    Ok(Response::from_parts(parts, Body::from(body)))
}
