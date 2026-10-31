use super::*;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::json;

#[derive(Debug)]
struct Recorded {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct Server {
    base: Url,
    records: Arc<Mutex<Vec<Recorded>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start(build: impl FnOnce(&str) -> Vec<(&'static str, u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let routes = build(base.as_str().trim_end_matches('/'));
        let records = Arc::new(Mutex::new(Vec::new()));
        let saved = Arc::clone(&records);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                        continue;
                    }
                    Err(error) => panic!("accept: {error}"),
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).unwrap();
                    head.extend(byte);
                }
                let head = String::from_utf8(head).unwrap();
                let mut first = head.lines().next().unwrap().split_whitespace();
                let method = first.next().unwrap().to_string();
                let target = first.next().unwrap().to_string();
                let headers: BTreeMap<_, _> = head
                    .lines()
                    .skip(1)
                    .filter_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        Some((key.to_lowercase(), value.trim().to_string()))
                    })
                    .collect();
                let length = headers
                    .get("content-length")
                    .map_or(0, |value| value.parse().unwrap());
                let mut body = vec![0; length];
                socket.read_exact(&mut body).unwrap();
                let path = target.split('?').next().unwrap();
                let (_, status, response) =
                    routes.iter().find(|(route, _, _)| *route == path).unwrap();
                let response = format!(
                    "HTTP/1.1 {status} Response\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
                saved.lock().unwrap().push(Recorded {
                    method,
                    target,
                    headers,
                    body,
                });
                socket.write_all(response.as_bytes()).unwrap();
            }
        });
        Self {
            base,
            records,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn local_http() -> Http {
    breakless::twitch::install_tls();
    http(Client::Tv).unwrap()
}

fn player(base: &str, manifest: &str) -> String {
    json!({"playabilityStatus": {"status": "OK"}, "streamingData": {"hlsManifestUrl": format!("{base}{manifest}")}}).to_string()
}

#[tokio::test]
async fn player_identity_is_sent_and_the_best_relative_variant_is_measured_once() {
    let server = Server::start(|base| {
        vec![
        ("/player", 200, player(base, "/playlists/master.m3u8")),
        ("/playlists/master.m3u8", 200, "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=10\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=20\n../best.m3u8\n".to_string()),
        ("/best.m3u8", 200, "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:8\n#EXTINF:2,live\n8.ts\n#EXT-X-DISCONTINUITY\n#EXT-X-DATERANGE:ID=\"range\",CLASS=\"observed\"\n#EXTINF:2,live\nhttps://cdn.example/9.ts\n".to_string()),
    ]
    });
    let (row, snapshots) = measure(
        &local_http(),
        &server.base.join("player").unwrap(),
        Client::Tv,
        "test_video1".to_string(),
        Duration::ZERO,
    )
    .await;
    assert!(row.answered && row.hls);
    assert_eq!(row.status, "OK");
    assert_eq!(row.errors, [] as [String; 0]);
    assert_eq!(
        (row.variants.len(), row.variants[0].bandwidth, row.polls),
        (2, 20, 1)
    );
    assert_eq!((row.counts.segments, row.counts.discontinuities), (2, 1));
    assert_eq!(row.counts.classes, BTreeSet::from(["observed".to_string()]));
    assert_eq!(
        row.counts.hosts,
        BTreeSet::from(["127.0.0.1".to_string(), "cdn.example".to_string()])
    );
    assert!(row.observations());
    assert!(
        snapshots
            .unwrap()
            .media
            .unwrap()
            .contains("#EXT-X-DISCONTINUITY")
    );
    let records = server.records.lock().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|r| r.method.as_str())
            .collect::<Vec<_>>(),
        ["POST", "GET", "GET"]
    );
    assert_eq!(
        records
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        ["/player", "/playlists/master.m3u8", "/best.m3u8"]
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&records[0].body).unwrap(),
        Client::Tv.player_body("test_video1")
    );
    assert_eq!(records[0].headers["x-youtube-client-name"], "7");
    assert_eq!(
        records[0].headers["x-youtube-client-version"],
        "7.20260707.07.00"
    );
    assert_eq!(
        records[0].headers["user-agent"],
        "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)"
    );
    assert_eq!(records[0].headers["content-type"], "application/json");
    assert!(!records[0].headers.contains_key("cookie"));
}

#[tokio::test]
async fn login_required_and_bot_status_are_reported_without_hls() {
    let server = Server::start(|_| {
        vec![("/player", 200, json!({"playabilityStatus": {"status": "LOGIN_REQUIRED", "reason": "Sign in to confirm you're not a bot"}}).to_string())]
    });
    let (row, snapshots) = measure(
        &local_http(),
        &server.base.join("player").unwrap(),
        Client::Tv,
        "test_video1".to_string(),
        Duration::ZERO,
    )
    .await;
    assert!(row.answered && row.login_required() && row.bot_check);
    assert!(!row.hls);
    assert_eq!(row.status, "LOGIN_REQUIRED");
    assert_eq!(row.errors, ["player: LOGIN_REQUIRED"]);
    assert!(snapshots.is_none());
}

#[tokio::test]
async fn signed_manifest_http_errors_do_not_disclose_the_url_or_response() {
    let server = Server::start(|base| {
        vec![
            (
                "/player",
                200,
                player(base, "/master?sig=private-signature&ip=198.51.100.91"),
            ),
            (
                "/master",
                403,
                "private-signature 198.51.100.91".to_string(),
            ),
        ]
    });
    let (row, snapshots) = measure(
        &local_http(),
        &server.base.join("player").unwrap(),
        Client::Tv,
        "test_video1".to_string(),
        Duration::ZERO,
    )
    .await;
    assert!(row.answered && row.hls);
    assert_eq!(row.errors, ["master: HTTP 403"]);
    assert!(snapshots.is_none());
}

#[tokio::test]
async fn a_direct_media_manifest_is_counted_without_an_extra_request() {
    let media = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:1\n#EXTINF:2,live\n1.ts\n";
    let server = Server::start(|base| {
        vec![
            ("/player", 200, player(base, "/direct.m3u8")),
            ("/direct.m3u8", 200, media.to_string()),
        ]
    });
    let (row, snapshots) = measure(
        &local_http(),
        &server.base.join("player").unwrap(),
        Client::Tv,
        "test_video1".to_string(),
        Duration::ZERO,
    )
    .await;
    assert_eq!(row.errors, [] as [String; 0]);
    assert_eq!(
        (row.variants.len(), row.counts.segments, row.polls),
        (0, 1, 1)
    );
    assert_eq!(snapshots.unwrap().media.as_deref(), Some(media));
    assert_eq!(server.records.lock().unwrap().len(), 2);
}

struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn capture_writes_only_named_scrubbed_master_and_media_files() {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let folder = Folder(std::env::temp_dir().join(format!(
        "breakless-youtube-probe-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1,NAME=\"ExampleChannel\"\nhttps://cdn.example/test_video1/stream?sig=private-signature&signature=private-signature&ip=198.51.100.91&expire=1799999999&id=test_video1\n";
    let snapshots = Snapshots {
        master: text.to_string(),
        media: Some(text.to_string()),
        identities: vec!["test_video1".to_string(), "ExampleChannel".to_string()],
    };
    capture(&folder.0, Client::WebSafari, 3, &snapshots).unwrap();
    let mut names = std::fs::read_dir(&folder.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "youtube-web_safari-3-master.m3u8",
            "youtube-web_safari-3-media.m3u8"
        ]
    );
    for name in names {
        let text = std::fs::read_to_string(folder.0.join(name)).unwrap();
        assert!(text.contains("somechannel"));
        for private in [
            "private-signature",
            "198.51.100.91",
            "1799999999",
            "test_video1",
            "ExampleChannel",
        ] {
            assert!(!text.contains(private), "{text}");
        }
    }
}

#[test]
fn duration_and_video_id_validation_rejects_invalid_measurements() {
    assert_eq!(minutes("0").unwrap(), Duration::ZERO);
    assert_eq!(minutes("0.5").unwrap(), Duration::from_secs(30));
    for invalid in ["NaN", "inf", "-1", "-inf", "1e300", "nonsense"] {
        assert!(minutes(invalid).is_err(), "{invalid}");
    }
    assert_eq!(video_id("aB_12-xyZ09").unwrap(), "aB_12-xyZ09");
    for invalid in ["short", "abcdefghijkl", "abcdefghij?", "ábcdefghij"] {
        assert!(video_id(invalid).is_err(), "{invalid}");
    }
}

#[tokio::test]
async fn a_success_status_with_html_bot_check_is_reported_without_a_player_answer() {
    let server = Server::start(|_| {
        vec![(
            "/player",
            200,
            "<html>Sign in to confirm you're not a bot</html>".to_string(),
        )]
    });
    let (row, snapshots) = measure(
        &local_http(),
        &server.base.join("player").unwrap(),
        Client::Tv,
        "test_video1".to_string(),
        Duration::ZERO,
    )
    .await;
    assert!(!row.answered);
    assert!(row.bot_check);
    assert_eq!(row.status, "NO_RESPONSE");
    assert_eq!(row.errors, ["player: bot check"]);
    assert!(snapshots.is_none());
}

#[tokio::test]
async fn a_forbidden_html_bot_check_keeps_the_http_status_without_disclosing_its_body() {
    let server = Server::start(|_| {
        vec![(
            "/player",
            403,
            "<html>Unusual traffic from 198.51.100.91; private-signature</html>".to_string(),
        )]
    });
    let (row, snapshots) = measure(
        &local_http(),
        &server.base.join("player?sig=private-signature").unwrap(),
        Client::Tv,
        "test_video1".to_string(),
        Duration::ZERO,
    )
    .await;
    assert!(!row.answered);
    assert!(row.bot_check);
    assert_eq!(row.status, "NO_RESPONSE");
    assert_eq!(row.errors, ["player: HTTP 403 (bot check)"]);
    assert!(snapshots.is_none());
}

#[tokio::test]
async fn polling_stops_at_its_deadline_when_a_media_server_never_answers() {
    use tokio::io::AsyncReadExt;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!("http://{}/media", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).await.unwrap();
        std::future::pending::<()>().await;
        drop(socket);
    });
    let mut row = Row::new("test_video1".to_string());
    let snapshot = tokio::time::timeout(
        Duration::from_secs(2),
        poll(
            &local_http(),
            &url,
            Duration::from_millis(20),
            None,
            &mut row,
        ),
    )
    .await
    .unwrap();
    server.abort();
    let _ = server.await;
    assert!(snapshot.is_none());
    assert_eq!(row.polls, 0);
    assert_eq!(row.errors, ["media: no playlist before deadline"]);
}
