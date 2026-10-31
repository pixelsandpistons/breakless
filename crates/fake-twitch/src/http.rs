//! Plain HTTP endpoints; a scripted drop returns no response bytes.

use std::io;

use axum::body::{Body, to_bytes};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use hyper::Request;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::task::{JoinHandle, JoinSet};

use crate::{FakeTwitch, Fault};

/// Serves the fake's routes until the returned task is aborted.
pub(crate) fn serve(listener: TcpListener, fake: FakeTwitch) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((socket, _)) = accepted else { break };
                    let fake = fake.clone();
                    connections.spawn(async move {
                        let service = service_fn(move |request| handle(request, fake.clone()));
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(socket), service)
                            .await;
                    });
                }
                _ = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    })
}

fn scripted(fault: Fault) -> io::Result<Option<Response>> {
    match fault {
        Fault::Pass => Ok(None),
        Fault::Drop => Err(io::Error::other("scripted dropped connection")),
        Fault::Status(code) => Ok(Some(
            (
                StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                "fake outage",
            )
                .into_response(),
        )),
    }
}

async fn handle(request: Request<Incoming>, fake: FakeTwitch) -> io::Result<Response> {
    let method = request.method().clone();
    let uri = request.uri().to_string();
    let path = request.uri().path().to_string();
    let via = request
        .headers()
        .get("via")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if method == Method::POST && path == "/gql" {
        return gql(request, &fake, &uri, via.as_deref()).await;
    }
    if method == Method::GET {
        if let Some(file) = path.strip_prefix("/api/channel/hls/") {
            let login = file.trim_end_matches(".m3u8").to_string();
            if let Some(response) = scripted(fake.record(
                method.as_str(),
                &uri,
                via.as_deref(),
                std::slice::from_ref(&login),
                format!("master {login}"),
            ))? {
                return Ok(response);
            }
            return Ok(fake.master(&login).map_or_else(
                || (StatusCode::NOT_FOUND, "Can not find channel").into_response(),
                playlist_response,
            ));
        }
        if let Some((login, file)) = path.strip_prefix("/media/").and_then(|p| p.split_once('/')) {
            let logins = [login.to_string()];
            if let Some(response) = scripted(fake.record(
                method.as_str(),
                &uri,
                via.as_deref(),
                &logins,
                format!("media {login}/{file}"),
            ))? {
                return Ok(response);
            }
            return Ok(fake.media(login).map_or_else(
                || (StatusCode::NOT_FOUND, "gone").into_response(),
                playlist_response,
            ));
        }
        if let Some(rest) = path.strip_prefix("/emotes/") {
            if let Some(response) = scripted(fake.record(
                method.as_str(),
                &uri,
                via.as_deref(),
                &[],
                format!("emotes {rest}"),
            ))? {
                return Ok(response);
            }
            let (code, value) = fake.emotes(rest);
            if code != 200
                && let Some(text) = value.as_str()
            {
                return Ok((
                    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                    text.to_string(),
                )
                    .into_response());
            }
            return Ok(json_response(code, &value));
        }
        if let Some((login, file)) = path
            .strip_prefix("/segments/")
            .and_then(|p| p.split_once('/'))
        {
            let logins = [login.to_string()];
            if let Some(response) = scripted(fake.record(
                method.as_str(),
                &uri,
                via.as_deref(),
                &logins,
                format!("segment {login}/{file}"),
            ))? {
                return Ok(response);
            }
            return Ok(([("Content-Type", "video/mp2t")], "fake segment").into_response());
        }
    }
    Ok((StatusCode::NOT_FOUND, "not found").into_response())
}

async fn gql(
    request: Request<Incoming>,
    fake: &FakeTwitch,
    uri: &str,
    via: Option<&str>,
) -> io::Result<Response> {
    let bytes = to_bytes(Body::new(request.into_body()), usize::MAX)
        .await
        .map_err(io::Error::other)?;
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut logins = Vec::new();
    if let Some(login) = body["variables"]["login"].as_str() {
        logins.push(login.to_lowercase());
    }
    logins.extend(
        body["variables"]["logins"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_lowercase),
    );
    if let Some(response) = scripted(fake.record("POST", uri, via, &logins, "gql".to_string()))? {
        return Ok(response);
    }
    Ok(json_response(200, &fake.gql(&body)))
}

fn playlist_response(text: String) -> Response {
    ([("Content-Type", "application/vnd.apple.mpegurl")], text).into_response()
}

fn json_response(code: u16, value: &Value) -> Response {
    (
        StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        [("Content-Type", "application/json")],
        value.to_string(),
    )
        .into_response()
}
