//! The HTTP surface. One handler matches the path by hand, as the prototype does, so the routes
//! and their answers stay exactly what `reference/python/server.py` lists.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Method, Response, StatusCode, Uri};
use serde_json::{Value, json};

use crate::app::App;
use crate::channel::Fault;
use crate::store::valid_login;
use crate::twitch::truncate;

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: https://static-cdn.jtvnw.net; \
media-src 'self' blob: https:; connect-src 'self' https:; worker-src 'self' blob:; \
frame-src https://www.twitch.tv; frame-ancestors 'self'; base-uri 'none'; form-action 'self'";
const M3U8: &str = "application/vnd.apple.mpegurl";
const JSON: &str = "application/json";

/// The router over shared state.
pub fn router(app: Arc<App>) -> Router {
    Router::new().fallback(dispatch).with_state(app)
}

fn send(code: StatusCode, body: impl Into<Vec<u8>>, ctype: &str, cache: &str) -> Response<Body> {
    let body: Vec<u8> = body.into();
    let mut r = Response::builder()
        .status(code)
        .header("Content-Type", ctype)
        .header("Content-Length", body.len().to_string())
        .header("Cache-Control", cache)
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer");
    if ctype.starts_with("text/html") {
        r = r.header("Content-Security-Policy", CSP);
    }
    r.body(Body::from(body))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

fn json(code: StatusCode, v: &Value) -> Response<Body> {
    send(code, v.to_string(), JSON, "no-store")
}

fn error(code: StatusCode, message: &str) -> Response<Body> {
    json(code, &json!({"error": message}))
}

fn fault(login: &str, f: &Fault) -> Response<Body> {
    match f {
        Fault::Offline => error(StatusCode::NOT_FOUND, &format!("{login} is not live")),
        Fault::Twitch(e) => error(StatusCode::BAD_GATEWAY, &truncate(&e.message, 200)),
    }
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn valid_slug(s: &str) -> bool {
    (1..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn valid_file(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

async fn dispatch(
    State(app): State<Arc<App>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    let path = uri.path();
    match method {
        Method::GET | Method::HEAD => {
            let mut r = get(&app, path, uri.query().unwrap_or("")).await;
            if method == Method::HEAD {
                *r.body_mut() = Body::empty();
            }
            r
        }
        Method::POST => post(&app, path, &headers, &body).await,
        _ => error(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"),
    }
}

async fn get(app: &App, path: &str, _query: &str) -> Response<Body> {
    if path == "/health" {
        return send(StatusCode::OK, "ok\n", "text/plain", "no-store");
    }
    if path == "/api/status" {
        return json(StatusCode::OK, &app.status().await);
    }
    if path == "/api/channels" {
        return json(StatusCode::OK, &app.listing().await);
    }
    if let Some(login) = path.strip_prefix("/api/watch/").filter(|l| valid_login(l)) {
        return json(StatusCode::OK, &app.watch(login).await);
    }
    if let Some(login) = path
        .strip_prefix("/api/chat/")
        .or_else(|| path.strip_prefix("/api/emotes/"))
        && valid_login(login)
    {
        return error(
            StatusCode::NOT_IMPLEMENTED,
            "chat is not yet in this build of the service",
        );
    }
    if let Some(rest) = path.strip_prefix("/live/") {
        if let Some(login) = rest.strip_suffix(".m3u8")
            && !login.contains('/')
            && valid_login(login)
        {
            return match app.channel(login).master(app).await {
                Ok(text) => send(StatusCode::OK, text, M3U8, "no-store"),
                Err(f) => fault(&login.to_lowercase(), &f),
            };
        }
        if let Some((login, file)) = rest.split_once('/')
            && let Some(slug) = file.strip_suffix(".m3u8")
            && valid_login(login)
            && valid_slug(slug)
        {
            return match app.channel(login).media(app, slug).await {
                Ok(text) => send(StatusCode::OK, text, M3U8, "no-store"),
                Err(f) => fault(&login.to_lowercase(), &f),
            };
        }
        return error(StatusCode::NOT_FOUND, "not found");
    }
    let file = if path == "/" || path.strip_prefix("/watch/").is_some_and(valid_login) {
        "index.html"
    } else if let Some(name) = path.strip_prefix("/static/").filter(|n| valid_file(n)) {
        name
    } else {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    match tokio::fs::read(app.config.web.join(file)).await {
        Ok(bytes) => send(StatusCode::OK, bytes, content_type(file), "no-cache"),
        Err(_) => error(StatusCode::NOT_FOUND, "not found"),
    }
}

async fn post(app: &App, path: &str, headers: &HeaderMap, body: &Bytes) -> Response<Body> {
    if body.len() > 2000 {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "too long");
    }
    // Only this page may change the list: the browser states where the request came from.
    let origin = headers
        .get("Origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let host = headers
        .get("Host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !origin.is_empty() && origin.rsplit("://").next().unwrap_or(origin) != host {
        return error(StatusCode::FORBIDDEN, "cross-site request refused");
    }
    let parsed: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(body).unwrap_or(Value::Null)
    };
    if !parsed.is_object() {
        return error(StatusCode::BAD_REQUEST, "not JSON");
    }
    if path == "/api/follow" {
        let login = parsed["login"].as_str().unwrap_or("").trim().to_lowercase();
        if !valid_login(&login) {
            return error(
                StatusCode::BAD_REQUEST,
                "a channel name is 2 to 25 letters, digits or _",
            );
        }
        let on = parsed.get("on").and_then(Value::as_bool).unwrap_or(true);
        return match app.store.set_follow(&login, on) {
            Ok(follows) => {
                app.forget_listing().await;
                json(StatusCode::OK, &json!({"ok": true, "follows": follows}))
            }
            Err(e) => error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("cannot write the channel list: {e}"),
            ),
        };
    }
    error(StatusCode::NOT_FOUND, "not found")
}
