//! The service against a fake Twitch: sessions, cutting, fallback, the list, the page.

mod fake_twitch;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use breakless::app::App;
use breakless::config::Config;
use breakless::routes::router;
use fake_twitch::FakeTwitch;
use serde_json::Value;

struct Rig {
    fake: FakeTwitch,
    base: String,
    data: PathBuf,
    http: reqwest::Client,
}

impl Rig {
    async fn start(proxies: &[&str]) -> Self {
        let fake = FakeTwitch::start().await;
        let data = std::env::temp_dir().join(format!(
            "breakless-test-{}",
            breakless::twitch::random_u64()
        ));
        std::fs::create_dir_all(&data).unwrap();
        let config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            proxy: proxies.iter().map(|p| (*p).to_string()).collect(),
            host: "test.local:8080".to_string(),
            data: data.clone(),
            web: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web"),
            gql_url: fake.gql_url(),
            usher_url: fake.usher_url(),
        };
        let app = Arc::new(App::new(config).unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router(app)).await.unwrap() });
        Self {
            fake,
            base: format!("http://{addr}"),
            data,
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    async fn get(&self, path: &str) -> (u16, String) {
        let r = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn get_full(&self, path: &str) -> reqwest::Response {
        self.http
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }

    async fn json(&self, path: &str) -> Value {
        let (code, text) = self.get(path).await;
        assert_eq!(code, 200, "{path}: {text}");
        serde_json::from_str(&text).unwrap()
    }

    async fn post(&self, path: &str, body: &str, origin: Option<&str>) -> (u16, Value) {
        let mut r = self
            .http
            .post(format!("{}{path}", self.base))
            .body(body.to_string());
        if let Some(o) = origin {
            r = r.header("Origin", o);
        }
        let r = r.send().await.unwrap();
        let code = r.status().as_u16();
        (
            code,
            serde_json::from_str(&r.text().await.unwrap()).unwrap_or(Value::Null),
        )
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.data);
    }
}

fn urls(playlist: &str) -> Vec<&str> {
    playlist.lines().filter(|l| l.starts_with("http")).collect()
}

#[tokio::test]
async fn health_answers() {
    let rig = Rig::start(&[]).await;
    assert_eq!(rig.get("/health").await, (200, "ok\n".to_string()));
}

#[tokio::test]
async fn a_live_channel_gets_a_master_with_our_addresses_and_a_home_session() {
    let rig = Rig::start(&[]).await;
    rig.fake.channel("alpha", &[], 5);
    let (code, text) = rig.get("/live/Alpha.m3u8").await;
    assert_eq!(code, 200, "{text}");
    assert!(text.contains("#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,CODECS=\"avc1.64002A,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\nalpha/chunked.m3u8\n"), "{text}");
    assert!(text.contains("GROUP-ID=\"720p60\",NAME=\"720p60\""));
    let status = rig.json("/api/status").await;
    assert_eq!(status["exit"]["configured"], false);
    assert_eq!(status["watching"][0]["channel"], "alpha");
    assert_eq!(status["watching"][0]["from"], "home");
    assert_eq!(status["watching"][0]["country"], "CA");
    assert_eq!(
        status["watching"][0]["qualities"],
        serde_json::json!(["1080p60 (source)", "720p60"])
    );
    assert_eq!(status["totals"]["sessions"]["home"], 1);
}

#[tokio::test]
async fn a_clean_stream_is_passed_on_and_grows_as_twitch_moves() {
    let rig = Rig::start(&[]).await;
    rig.fake.channel("beta", &[], 5);
    let (code, first) = rig.get("/live/beta/chunked.m3u8").await;
    assert_eq!(code, 200, "{first}");
    assert_eq!(urls(&first).len(), 5);
    assert!(first.contains("#EXT-X-MEDIA-SEQUENCE:0\n"));
    assert!(first.ends_with("http://cdn.example/beta/live-4.ts\n"));
    rig.fake.advance("beta", 2);
    tokio::time::sleep(std::time::Duration::from_millis(950)).await; // past the 0.9 s reuse
    let (_, second) = rig.get("/live/beta/chunked.m3u8").await;
    assert_eq!(urls(&second).len(), 7);
    assert!(second.ends_with("http://cdn.example/beta/live-6.ts\n"));
    assert!(!second.contains("#EXT-X-DISCONTINUITY\n"));
    let status = rig.json("/api/status").await;
    assert_eq!(status["watching"][0]["segments_passed"], 7);
    assert_eq!(
        status["watching"][0]["asked"],
        serde_json::json!(["chunked"])
    );
    assert_eq!(status["totals"]["stream_segments_passed"], 7);
}

#[tokio::test]
async fn a_preroll_is_cut_the_playlist_holds_still_and_the_break_is_recorded() {
    let rig = Rig::start(&[]).await;
    rig.fake
        .channel("gamma", &[true, true, true, true, true, true, true], 5); // 7 advert segments, then stream
    let (code, first) = rig.get("/live/gamma/chunked.m3u8").await;
    assert_eq!(code, 200, "{first}");
    assert!(urls(&first).is_empty(), "nothing to show yet: {first}");
    let watch = rig.json("/api/watch/gamma").await;
    assert_eq!(watch["advert_break"]["roll"], "preroll");
    assert_eq!(watch["advert_break"]["seconds"], "10");
    assert_eq!(watch["segments_cut"], 5);
    // two more advert segments, then three of stream come into view
    rig.fake.advance("gamma", 5);
    tokio::time::sleep(std::time::Duration::from_millis(950)).await;
    let (_, second) = rig.get("/live/gamma/chunked.m3u8").await;
    assert_eq!(
        urls(&second),
        [
            "http://cdn.example/gamma/live-7.ts",
            "http://cdn.example/gamma/live-8.ts",
            "http://cdn.example/gamma/live-9.ts"
        ]
    );
    assert!(!second.contains("ad-"));
    let status = rig.json("/api/status").await;
    assert_eq!(status["totals"]["advert_segments_cut"]["home"], 7);
    assert_eq!(status["totals"]["advert_breaks"]["home"], 1);
    assert!(status["watching"][0]["advert_break"].is_null());
    let log = std::fs::read_to_string(rig.data.join("adverts.jsonl")).unwrap();
    let record: Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(record["channel"], "gamma");
    assert_eq!(record["from"], "home");
    assert_eq!(record["roll"], "preroll");
    // the record counts what was cut while the break was under way; the fetch that ends the
    // break (two more advert segments, then stream) is not added, as in the prototype. The totals
    // above count all seven.
    assert_eq!(record["segments_cut"], 5);
    assert_eq!(record["advertised_seconds"], "10");
}

#[tokio::test]
async fn a_midroll_is_cut_with_a_discontinuity_after_it() {
    let rig = Rig::start(&[]).await;
    let mut ads = vec![false; 5];
    ads.extend([true, true, true]);
    rig.fake.channel("delta", &ads, 5);
    rig.get("/live/delta/chunked.m3u8").await;
    rig.fake.advance("delta", 3); // the three advert segments
    tokio::time::sleep(std::time::Duration::from_millis(950)).await;
    let (_, during) = rig.get("/live/delta/chunked.m3u8").await;
    assert_eq!(urls(&during).len(), 5, "held: {during}");
    rig.fake.advance("delta", 2); // stream again
    tokio::time::sleep(std::time::Duration::from_millis(950)).await;
    let (_, after) = rig.get("/live/delta/chunked.m3u8").await;
    assert_eq!(after.matches("#EXT-X-DISCONTINUITY\n").count(), 1);
    assert!(after.find("#EXT-X-DISCONTINUITY\n").unwrap() < after.find("live-8.ts").unwrap());
    assert!(!after.contains("ad-"));
    let status = rig.json("/api/status").await;
    assert_eq!(status["totals"]["advert_segments_cut"]["home"], 3);
}

#[tokio::test]
async fn an_offline_channel_is_404_with_a_plain_message() {
    let rig = Rig::start(&[]).await;
    rig.fake.offline("nobody");
    let (code, text) = rig.get("/live/nobody.m3u8").await;
    assert_eq!(code, 404);
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["error"],
        "nobody is not live"
    );
    let (code, _) = rig.get("/live/unknown_name/chunked.m3u8").await;
    assert_eq!(code, 404);
}

#[tokio::test]
async fn a_passing_error_is_retried_once_without_losing_the_session() {
    let rig = Rig::start(&[]).await;
    rig.fake.channel("eps", &[], 5);
    rig.fake.fail_media_next(1);
    let (code, text) = rig.get("/live/eps/chunked.m3u8").await;
    assert_eq!(code, 200, "{text}");
    assert_eq!(urls(&text).len(), 5);
    let status = rig.json("/api/status").await;
    assert_eq!(
        status["totals"]["sessions"]["home"], 1,
        "the session was kept"
    );
}

#[tokio::test]
async fn a_dead_exit_sends_the_session_home_and_shows_in_status() {
    let rig = Rig::start(&["http://127.0.0.1:9"]).await;
    rig.fake.channel("zeta", &[], 5);
    let (code, text) = rig.get("/live/zeta/chunked.m3u8").await;
    assert_eq!(code, 200, "{text}");
    assert_eq!(urls(&text).len(), 5);
    let status = rig.json("/api/status").await;
    assert_eq!(status["exit"]["configured"], true);
    assert_eq!(status["exit"]["usable"], false);
    assert_eq!(status["exit"]["proxy"], "127.0.0.1:9");
    assert!(status["exit"]["last_error"].as_str().unwrap().len() > 20);
    assert_eq!(status["watching"][0]["from"], "home");
    assert_eq!(status["totals"]["sessions"]["home"], 1);
    assert_eq!(status["totals"]["sessions"]["exit"], 0);
    assert_eq!(status["exits"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn follows_are_written_listed_and_guarded() {
    let rig = Rig::start(&[]).await;
    rig.fake.channel("live_one", &[], 5);
    let (code, v) = rig
        .post("/api/follow", r#"{"login": "Live_One", "on": true}"#, None)
        .await;
    assert_eq!((code, v["ok"].as_bool()), (200, Some(true)));
    let (code, _) = rig
        .post("/api/follow", r#"{"login": "quiet_one"}"#, None)
        .await;
    assert_eq!(code, 200);
    assert_eq!(
        std::fs::read_to_string(rig.data.join("follows.json")).unwrap(),
        r#"["live_one","quiet_one"]"#
    );
    let list = rig.json("/api/channels").await;
    assert_eq!(list["host"], "test.local:8080");
    assert!(list["error"].is_null());
    assert_eq!(list["follows"][0]["login"], "live_one");
    assert_eq!(list["follows"][0]["live"], true);
    assert_eq!(list["follows"][0]["viewers"], 100);
    assert_eq!(
        list["follows"][1],
        serde_json::json!({"login": "quiet_one", "live": false, "name": "quiet_one"})
    );
    assert_eq!(list["top"][0]["login"], "live_one");
    let (code, v) = rig.post("/api/follow", r#"{"login": "x"}"#, None).await;
    assert_eq!(code, 400, "{v}");
    let (code, v) = rig
        .post(
            "/api/follow",
            r#"{"login": "quiet_one", "on": false}"#,
            Some("https://evil.example"),
        )
        .await;
    assert_eq!(
        (code, v["error"].as_str()),
        (403, Some("cross-site request refused"))
    );
    let (code, _) = rig.post("/api/follow", "not json", None).await;
    assert_eq!(code, 400);
    let (code, _) = rig.post("/api/follow", &"x".repeat(2001), None).await;
    assert_eq!(code, 413);
}

#[tokio::test]
async fn the_page_and_its_files_are_served_with_the_security_headers() {
    let rig = Rig::start(&[]).await;
    let r = rig.get_full("/").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "text/html; charset=utf-8");
    assert!(
        r.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .starts_with("default-src 'self'")
    );
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    let r = rig.get_full("/watch/somebody").await;
    assert_eq!(r.status(), 200);
    let r = rig.get_full("/static/app.js").await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(r.headers()["cache-control"], "no-cache");
    assert!(!r.headers().contains_key("content-security-policy"));
    assert_eq!(rig.get("/static/../Cargo.toml").await.0, 404);
    assert_eq!(rig.get("/static/missing.css").await.0, 404);
    assert_eq!(rig.get("/nothing/here").await.0, 404);
    let (code, text) = rig.get("/api/chat/somebody").await;
    assert_eq!(code, 501, "{text}");
}
