//! The fake's public controls against its loopback HTTP, proxy and IRC sockets.

use std::net::SocketAddr;
use std::time::Duration;

use breakless_formats::{master_info, parse_media, variants};
use fake_twitch::FakeTwitch;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

async fn request(
    address: SocketAddr,
    method: &str,
    target: &str,
    body: &str,
    via: Option<&str>,
) -> Option<(u16, String)> {
    let mut socket = TcpStream::connect(address).await.unwrap();
    let via = via.map_or_else(String::new, |value| format!("Via: {value}\r\n"));
    let message = format!(
        "{method} {target} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n{via}\r\n{body}",
        body.len()
    );
    socket.write_all(message.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    if let Err(error) = tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut bytes))
        .await
        .unwrap()
    {
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    }
    if bytes.is_empty() {
        return None;
    }
    let header_end = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap();
    let head = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let status = head
        .split_ascii_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let body = &bytes[header_end + 4..];
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut body = body;
        let mut decoded = Vec::new();
        loop {
            let end = body.windows(2).position(|part| part == b"\r\n").unwrap();
            let size =
                usize::from_str_radix(std::str::from_utf8(&body[..end]).unwrap(), 16).unwrap();
            body = &body[end + 2..];
            if size == 0 {
                break;
            }
            decoded.extend_from_slice(&body[..size]);
            body = &body[size + 2..];
        }
        decoded
    } else {
        body.to_vec()
    };
    Some((status, String::from_utf8(body).unwrap()))
}

async fn get(url: &str) -> (u16, String) {
    let (address, path) = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    request(
        address.parse().unwrap(),
        "GET",
        &format!("/{path}"),
        "",
        None,
    )
    .await
    .unwrap()
}

async fn media_url(fake: &FakeTwitch, login: &str) -> String {
    let (status, master) = get(&fake.usher_url().replace("{login}", login)).await;
    assert_eq!(status, 200);
    variants(&master)[0].url.clone()
}

#[tokio::test]
async fn masters_offer_parseable_variants_on_the_loopback_cdn_and_channel_country() {
    let fake = FakeTwitch::start().await;
    assert!(fake.gql.ip().is_loopback());
    assert!(fake.usher.ip().is_loopback());
    assert!(fake.cdn.ip().is_loopback());
    assert_ne!(fake.gql, fake.usher);
    assert_ne!(fake.usher, fake.cdn);
    let channel = fake.channel("alpha");
    channel.segment_interval(Duration::ZERO);
    channel.country("IS");
    let (status, master) = get(&fake.usher_url().replace("{login}", "alpha")).await;
    assert_eq!(status, 200);
    assert_eq!(master_info(&master).country, "IS");
    let qualities = variants(&master);
    assert_eq!(qualities.len(), 2);
    assert_eq!(qualities[0].slug, "chunked");
    assert_eq!(qualities[1].slug, "720p60");
    for quality in qualities {
        assert!(quality.url.starts_with(&format!("http://{}/", fake.cdn)));
        let (status, text) = get(&quality.url).await;
        assert_eq!(status, 200);
        let playlist = parse_media(&text);
        assert_eq!(playlist.segments.len(), 5);
        assert!(playlist.segments.iter().all(|segment| !segment.ad));
        assert!(
            playlist
                .segments
                .iter()
                .all(|segment| segment.pdt.is_some())
        );
        assert_eq!(get(&playlist.segments[0].url).await.0, 200);
    }
}

#[tokio::test]
async fn windows_advance_with_the_clock_and_keep_five_two_second_segments() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("clock");
    channel.segment_interval(Duration::from_millis(10));
    let url = media_url(&fake, "clock").await;
    let first = parse_media(&get(&url).await.1);
    let later = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let playlist = parse_media(&get(&url).await.1);
            if playlist.seq > first.seq {
                break playlist;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(later.segments.len(), 5);
    for pair in later.segments.windows(2) {
        assert_eq!(pair[1].n, pair[0].n + 1);
        assert!((pair[1].when.unwrap() - pair[0].when.unwrap() - 2.0).abs() < f64::EPSILON);
    }
    assert!(
        later
            .segments
            .iter()
            .all(|segment| (segment.dur - 2.0).abs() < f64::EPSILON)
    );
}

#[tokio::test]
async fn advert_break_is_all_adverts_for_its_segments_then_clean_with_a_discontinuity() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("adverts");
    channel.segment_interval(Duration::ZERO);
    let url = media_url(&fake, "adverts").await;
    channel.advert_break(3, "midroll");
    for expected in 1..=3 {
        let text = get(&url).await.1;
        let playlist = parse_media(&text);
        assert_eq!(playlist.segments.len(), expected);
        assert!(playlist.segments.iter().all(|segment| segment.ad));
        assert!(
            playlist
                .segments
                .iter()
                .all(|segment| segment.title.starts_with("Amazon|"))
        );
        assert!(text.contains("CLASS=\"twitch-stitched-ad\""));
        let windows_only = text
            .lines()
            .map(|line| {
                if line.starts_with("#EXTINF:") {
                    "#EXTINF:2.000,live"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            parse_media(&windows_only)
                .segments
                .iter()
                .all(|segment| segment.ad)
        );
        assert_eq!(playlist.ads.unwrap().roll, "midroll");
        channel.advance(1);
    }
    let clean = parse_media(&get(&url).await.1);
    assert_eq!(clean.segments.len(), 1);
    assert!(clean.segments.iter().all(|segment| !segment.ad));
    assert!(clean.segments[0].disc);
    assert!(clean.ads.is_none());
}

#[tokio::test]
async fn channel_faults_drop_exactly_the_next_requests_and_status_counts_expire() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("faults");
    let media = media_url(&fake, "faults").await;
    channel.drop_next(2);
    let path = "/api/channel/hls/faults.m3u8";
    let token =
        json!({"query": "PlaybackAccessToken", "variables": {"login": "faults"}}).to_string();
    assert_eq!(request(fake.gql, "POST", "/gql", &token, None).await, None);
    assert_eq!(request(fake.usher, "GET", path, "", None).await, None);
    assert_eq!(
        request(fake.usher, "GET", path, "", None).await.unwrap().0,
        200
    );
    channel.status(503, 2);
    assert_eq!(
        request(fake.usher, "GET", path, "", None).await.unwrap().0,
        503
    );
    assert_eq!(get(&media).await.0, 503);
    assert_eq!(
        request(fake.usher, "GET", path, "", None).await.unwrap().0,
        200
    );
    channel.offline();
    assert_eq!(
        request(fake.usher, "GET", path, "", None).await.unwrap().0,
        404
    );
}

#[tokio::test]
async fn request_records_keep_methods_uris_and_via_in_arrival_order() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("recorded");
    let token =
        json!({"query": "PlaybackAccessToken", "variables": {"login": "recorded"}}).to_string();
    let response = request(fake.gql, "POST", "/gql", &token, Some("1.0 previous"))
        .await
        .unwrap();
    assert_eq!(response.0, 200);
    let value: Value = serde_json::from_str(&response.1).unwrap();
    assert_eq!(
        value["data"]["streamPlaybackAccessToken"]["value"],
        "tok-recorded"
    );
    request(
        fake.usher,
        "GET",
        "/api/channel/hls/recorded.m3u8",
        "",
        None,
    )
    .await
    .unwrap();
    let records = channel.requests();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].method, "POST");
    assert!(records[0].uri.ends_with("/gql"));
    assert_eq!(records[0].via.as_deref(), Some("1.0 previous"));
    assert_eq!(records[1].method, "GET");
    assert!(records[1].uri.ends_with("/api/channel/hls/recorded.m3u8"));
    assert_eq!(records[1].via, None);
}

#[tokio::test]
async fn the_proxy_forwards_absolute_requests_adds_via_and_recovers_after_down() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("proxied");
    let proxy = fake.proxy().await;
    let target = fake.usher_url().replace("{login}", "proxied");
    let forwarded = request(proxy.addr, "GET", &target, "", None).await.unwrap();
    assert_eq!(forwarded.0, 200);
    assert_eq!(variants(&forwarded.1).len(), 2);
    let records = channel.requests();
    assert_eq!(records.len(), 1);
    assert!(records[0].via.as_ref().is_some_and(|via| !via.is_empty()));
    proxy.down();
    assert_eq!(request(proxy.addr, "GET", &target, "", None).await, None);
    assert_eq!(channel.requests().len(), 1);
    proxy.up();
    assert_eq!(
        request(proxy.addr, "GET", &target, "", None)
            .await
            .unwrap()
            .0,
        200
    );
    assert_eq!(channel.requests().len(), 2);
    assert_eq!(proxy.url(), format!("http://{}", proxy.addr));
}

#[tokio::test]
async fn irc_join_answers_and_broadcasts_while_recording_client_lines() {
    let fake = FakeTwitch::start().await;
    let mut socket = TcpStream::connect(fake.irc_url().strip_prefix("tcp://").unwrap())
        .await
        .unwrap();
    socket.write_all(b"JOIN #chat\r\n").await.unwrap();
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert!(line.contains(" 001 "));
    line.clear();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert!(line.contains("ROOMSTATE #chat"));
    assert!(line.contains("room-id=71092938"));
    assert_eq!(fake.irc_received(), ["JOIN #chat"]);
    fake.irc_say("PING :fixture");
    line.clear();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(line, "PING :fixture\r\n");
}

#[tokio::test]
async fn emote_tables_and_status_controls_keep_the_existing_routes() {
    let fake = FakeTwitch::start().await;
    let path = "api.betterttv.net/3/cached/emotes/global";
    let (status, text) = get(&format!("{}/{path}", fake.emotes_url())).await;
    assert_eq!(status, 200);
    let table: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(table[0]["code"], ":tf:");
    assert_eq!(fake.emote_requests(), [path]);
    fake.emote_status("betterttv.net", 503);
    assert_eq!(get(&format!("{}/{path}", fake.emotes_url())).await.0, 503);
    assert_eq!(get(&format!("{}/unknown", fake.emotes_url())).await.0, 404);
}

async fn read_sized_response(reader: &mut BufReader<TcpStream>) -> (u16, String) {
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let status = line
        .split_ascii_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut length = None;
    loop {
        line.clear();
        assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("the proxy forwards a sized playlist body")];
    reader.read_exact(&mut body).await.unwrap();
    (status, String::from_utf8(body).unwrap())
}

#[tokio::test]
async fn proxy_down_closes_a_persistent_connection_and_up_accepts_a_new_one() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("persistent");
    let proxy = fake.proxy().await;
    let target = fake.usher_url().replace("{login}", "persistent");
    let socket = TcpStream::connect(proxy.addr).await.unwrap();
    let mut reader = BufReader::new(socket);
    let message = format!(
        "GET {target} HTTP/1.1\r\nHost: {}\r\nConnection: keep-alive\r\n\r\n",
        fake.usher
    );
    for _ in 0..2 {
        reader
            .get_mut()
            .write_all(message.as_bytes())
            .await
            .unwrap();
        let (status, body) =
            tokio::time::timeout(Duration::from_secs(2), read_sized_response(&mut reader))
                .await
                .unwrap();
        assert_eq!(status, 200);
        assert_eq!(variants(&body).len(), 2);
    }
    assert_eq!(channel.requests().len(), 2);
    proxy.down();
    let mut byte = [0];
    match tokio::time::timeout(Duration::from_secs(2), reader.read(&mut byte))
        .await
        .unwrap()
    {
        Ok(count) => assert_eq!(count, 0),
        Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset),
    }
    proxy.up();
    assert_eq!(
        request(proxy.addr, "GET", &target, "", None)
            .await
            .unwrap()
            .0,
        200
    );
    assert_eq!(channel.requests().len(), 3);
}
