//! The service against a fake Twitch: sessions, cutting, fallback, the list, the page.

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
        Self::with_fake(fake, proxies).await
    }

    async fn with_fake(fake: FakeTwitch, proxies: &[&str]) -> Self {
        Self::with_fake_rest(fake, proxies, 30).await
    }

    /// `exit_rest` is the seconds a failed exit is left alone; the service's default is 30.
    async fn with_fake_rest(fake: FakeTwitch, proxies: &[&str], exit_rest: u64) -> Self {
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
            irc_url: fake.irc_url(),
            emotes_url: fake.emotes_url(),
            exit_rest,
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
    rig.fake.channel("alpha").script(&[], 5);
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
    rig.fake.channel("beta").script(&[], 5);
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
        .channel("gamma")
        .script(&[true, true, true, true, true, true, true], 5); // 7 advert segments, then stream
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
    rig.fake.channel("delta").script(&ads, 5);
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
    rig.fake.channel("eps").script(&[], 5);
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
    rig.fake.channel("zeta").script(&[], 5);
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
async fn a_live_exit_carries_playlists_and_reports_the_country_twitch_saw() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("live_exit");
    channel.segment_interval(std::time::Duration::ZERO);
    channel.country("IS");
    let proxy = fake.proxy().await;
    let rig = Rig::with_fake(fake, &[&proxy.url()]).await;
    let (code, text) = rig.get("/live/live_exit/chunked.m3u8").await;
    assert_eq!(code, 200, "{text}");
    assert_eq!(urls(&text).len(), 5);
    let records = channel.requests();
    let playlists: Vec<_> = records
        .iter()
        .filter(|request| {
            request.uri.starts_with("/api/channel/hls/") || request.uri.starts_with("/media/")
        })
        .map(|request| {
            (
                request.method.as_str(),
                request.uri.split('?').next().unwrap(),
                request.via.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        playlists,
        [
            (
                "GET",
                "/api/channel/hls/live_exit.m3u8",
                Some("1.1 fake-twitch")
            ),
            (
                "GET",
                "/media/live_exit/chunked.m3u8",
                Some("1.1 fake-twitch")
            )
        ]
    );
    let status = rig.json("/api/status").await;
    assert_eq!(status["exit"]["usable"], true);
    assert_eq!(status["exit"]["country"], "IS");
    assert_eq!(status["exit"]["cluster"], "fake_cluster");
    assert_eq!(status["exit"]["last_error"], "");
    assert_eq!(status["watching"][0]["from"], "exit");
    assert_eq!(status["watching"][0]["country"], "IS");
    assert_eq!(status["totals"]["sessions"]["exit"], 1);
    assert_eq!(status["totals"]["sessions"]["home"], 0);
}

#[tokio::test]
async fn an_exit_that_goes_down_during_a_session_sends_the_playlist_home() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("exit_down");
    channel.segment_interval(std::time::Duration::ZERO);
    channel.country("IS");
    let proxy = fake.proxy().await;
    let rig = Rig::with_fake(fake, &[&proxy.url()]).await;
    let path = "/live/exit_down/chunked.m3u8";
    let (code, first) = rig.get(path).await;
    assert_eq!(code, 200, "{first}");
    assert_eq!(urls(&first).len(), 5);
    let before = rig.json("/api/status").await;
    assert_eq!(before["watching"][0]["from"], "exit");
    assert_eq!(before["totals"]["sessions"]["exit"], 1);
    assert_eq!(before["totals"]["sessions"]["home"], 0);
    proxy.down();
    // Let the service's media cache expire before asking the failed exit again.
    tokio::time::sleep(std::time::Duration::from_millis(950)).await;
    let (code, text) = rig.get(path).await;
    assert_eq!(code, 200, "{text}");
    assert_eq!(urls(&text).len(), 5);
    let status = rig.json("/api/status").await;
    assert_eq!(status["watching"][0]["from"], "home");
    assert_eq!(status["totals"]["sessions"]["home"], 1);
    assert_eq!(status["totals"]["sessions"]["exit"], 1);
    assert_eq!(status["exit"]["usable"], false);
    assert_ne!(status["exit"]["last_error"].as_str().unwrap(), "");
}

#[tokio::test]
async fn an_exit_that_comes_back_after_its_rest_gets_the_session_again() {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("exit_back");
    channel.segment_interval(std::time::Duration::ZERO);
    channel.country("IS");
    let proxy = fake.proxy().await;
    let rig = Rig::with_fake_rest(fake, &[&proxy.url()], 1).await;
    let path = "/live/exit_back/chunked.m3u8";
    let (code, text) = rig.get(path).await;
    assert_eq!(code, 200, "{text}");
    proxy.down();
    // The playlist is refreshed from Twitch at most every 0.9 s; wait that out before the exit
    // is asked and found dead.
    tokio::time::sleep(std::time::Duration::from_millis(950)).await;
    let (code, text) = rig.get(path).await;
    assert_eq!(code, 200, "{text}");
    let down = rig.json("/api/status").await;
    assert_eq!(down["watching"][0]["from"], "home");
    assert_eq!(down["exit"]["usable"], false);
    proxy.up();
    // The exit rests 1 s in this rig; the next refresh after that goes back through it.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let (code, text) = rig.get(path).await;
    assert_eq!(code, 200, "{text}");
    assert_eq!(urls(&text).len(), 5);
    let back = rig.json("/api/status").await;
    assert_eq!(back["watching"][0]["from"], "exit");
    assert_eq!(back["watching"][0]["country"], "IS");
    assert_eq!(back["exit"]["usable"], true);
    assert_eq!(back["totals"]["sessions"]["exit"], 2);
    assert_eq!(back["totals"]["sessions"]["home"], 1);
}

#[tokio::test]
async fn a_dropped_media_request_is_retried_once_without_losing_the_home_session() {
    let rig = Rig::start(&[]).await;
    let channel = rig.fake.channel("dropped_media");
    channel.segment_interval(std::time::Duration::ZERO);
    let (code, master) = rig.get("/live/dropped_media.m3u8").await;
    assert_eq!(code, 200, "{master}");
    channel.drop_next(1);
    let (code, text) = rig.get("/live/dropped_media/chunked.m3u8").await;
    assert_eq!(code, 200, "{text}");
    assert_eq!(urls(&text).len(), 5);
    let records = channel.requests();
    let media: Vec<_> = records
        .iter()
        .filter(|request| request.uri.starts_with("/media/"))
        .map(|request| {
            (
                request.method.as_str(),
                request.uri.as_str(),
                request.via.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        media,
        [("GET", "/media/dropped_media/chunked.m3u8", None); 2]
    );
    let status = rig.json("/api/status").await;
    assert_eq!(status["watching"][0]["from"], "home");
    assert_eq!(status["totals"]["sessions"]["home"], 1);
    assert_eq!(status["totals"]["sessions"]["exit"], 0);
}

#[tokio::test]
async fn a_dropped_token_request_through_the_exit_is_asked_once_more_and_the_session_still_opens_there()
 {
    let fake = FakeTwitch::start().await;
    let channel = fake.channel("dropped_token");
    channel.segment_interval(std::time::Duration::ZERO);
    let proxy = fake.proxy().await;
    let rig = Rig::with_fake(fake, &[&proxy.url()]).await;
    channel.drop_next(1);
    let (code, master) = rig.get("/live/dropped_token.m3u8").await;
    assert_eq!(code, 200, "{master}");
    let records = channel.requests();
    let asked: Vec<_> = records
        .iter()
        .map(|request| {
            (
                request.method.as_str(),
                request.uri.split('?').next().unwrap(),
                request.via.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        asked,
        [
            ("POST", "/gql", Some("1.1 fake-twitch")),
            ("POST", "/gql", Some("1.1 fake-twitch")),
            (
                "GET",
                "/api/channel/hls/dropped_token.m3u8",
                Some("1.1 fake-twitch")
            )
        ]
    );
    let (code, text) = rig.get("/live/dropped_token/chunked.m3u8").await;
    assert_eq!(code, 200, "{text}");
    assert_eq!(urls(&text).len(), 5);
    let status = rig.json("/api/status").await;
    assert_eq!(status["watching"][0]["from"], "exit");
    assert_eq!(status["totals"]["sessions"]["exit"], 1);
    assert_eq!(status["totals"]["sessions"]["home"], 0);
    assert_eq!(status["exit"]["usable"], true);
    assert_eq!(status["exit"]["last_error"], "");
}

#[tokio::test]
async fn follows_are_written_listed_and_guarded() {
    let rig = Rig::start(&[]).await;
    rig.fake.channel("live_one").script(&[], 5);
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
}

const MSG: &str = "@badge-info=subscriber/14;badges=moderator/1,subscriber/12,bits/1000;color=#1E90FF;display-name=Some\\sOne;\
emotes=25:6-10,18-22/1902:12-16;first-msg=0;id=abc-1;room-id=71092938;tmi-sent-ts=1;user-id=7 \
:someone!someone@someone.tmi.twitch.tv PRIVMSG #chan :hello Kappa Keepo Kappa: done";

async fn joined(rig: &Rig, login: &str) -> Value {
    for _ in 0..50 {
        let v = rig.json(&format!("/api/chat/{login}")).await;
        if v["state"] == "joined" {
            return v;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("{login}: chat never joined");
}

#[tokio::test]
async fn chat_joins_as_a_guest_numbers_events_and_answers_pings() {
    let rig = Rig::start(&[]).await;
    let first = joined(&rig, "Chan").await;
    assert_eq!(first["channel"], "chan");
    assert_eq!(first["room_id"], "71092938");
    assert_eq!(first["seq"], 0);
    assert_eq!(first["events"], serde_json::json!([]));
    let sent = rig.fake.irc_received();
    assert_eq!(sent[0], "CAP REQ :twitch.tv/tags twitch.tv/commands");
    assert!(sent[1].starts_with("NICK justinfan"), "{sent:?}");
    assert_eq!(sent[2], "JOIN #chan");
    rig.fake.irc_say(MSG);
    rig.fake.irc_say(MSG);
    let got = rig.json("/api/chat/chan?after=0&wait=5").await;
    assert_eq!(got["seq"], 2);
    let events = got["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["n"], 1);
    assert_eq!(events[0]["k"], "m");
    assert_eq!(events[0]["name"], "Some One");
    assert_eq!(events[0]["emotes"][0], serde_json::json!(["25", 6, 10]));
    assert_eq!(events[1]["n"], 2);
    let later = rig.json("/api/chat/chan?after=1").await;
    assert_eq!(later["events"].as_array().unwrap().len(), 1);
    assert_eq!(later["events"][0]["n"], 2);
    rig.fake.irc_say("PING :tmi.twitch.tv");
    for _ in 0..50 {
        if rig
            .fake
            .irc_received()
            .contains(&"PONG :tmi.twitch.tv".to_string())
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        rig.fake
            .irc_received()
            .contains(&"PONG :tmi.twitch.tv".to_string())
    );
    let status = rig.json("/api/status").await;
    assert_eq!(status["chat"][0]["channel"], "chan");
    assert_eq!(status["chat"][0]["state"], "joined");
    assert_eq!(status["chat"][0]["events"], 2);
}

#[tokio::test]
async fn a_poll_waits_for_the_next_message_and_gives_up_after_wait() {
    let rig = Rig::start(&[]).await;
    joined(&rig, "waiting").await;
    let t0 = std::time::Instant::now();
    let empty = rig.json("/api/chat/waiting?after=0&wait=0.3").await;
    assert_eq!(empty["events"], serde_json::json!([]));
    assert!(t0.elapsed() >= std::time::Duration::from_millis(250));
    let fake = rig.fake.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        fake.irc_say(MSG);
    });
    let t0 = std::time::Instant::now();
    let got = rig.json("/api/chat/waiting?after=0&wait=5").await;
    assert_eq!(got["events"].as_array().unwrap().len(), 1);
    assert!(t0.elapsed() < std::time::Duration::from_secs(2));
    // a number from before a restart gets the recent ones
    let again = rig.json("/api/chat/waiting?after=5000").await;
    assert_eq!(again["events"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn emotes_merge_channel_over_global_and_a_provider_away_does_not_empty_the_rest() {
    let rig = Rig::start(&[]).await;
    joined(&rig, "emoting").await;
    let r = rig.get_full("/api/emotes/emoting").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["cache-control"], "private, max-age=300");
    let got: Value = serde_json::from_str(&r.text().await.unwrap()).unwrap();
    assert_eq!(got["room_id"], "71092938");
    assert_eq!(
        got["emotes"]["catJAM"],
        serde_json::json!(["7", "01C", false])
    );
    assert_eq!(got["emotes"]["OnlyFFZ"][0], "f");
    assert_eq!(got["emotes"]["monkaS"][0], "b");
    assert_eq!(
        got["counts"]["b"],
        serde_json::json!({"global": 1, "channel": 2})
    );
    assert_eq!(got["errors"], serde_json::json!({}));
    assert_eq!(rig.fake.emote_requests().len(), 6);
    // answers are reused
    rig.json("/api/emotes/emoting").await;
    assert_eq!(rig.fake.emote_requests().len(), 6);
    // a fresh service: 7TV away, FrankerFaceZ has no room (a 404 is not a fault)
    let rig = Rig::start(&[]).await;
    rig.fake.emote_status("7tv.io", 500);
    rig.fake.emote_status("frankerfacez.com/v1/room", 404);
    joined(&rig, "emoting").await;
    let got = rig.json("/api/emotes/emoting").await;
    assert_eq!(
        got["emotes"]["catJAM"],
        serde_json::json!(["b", "aaa", false])
    );
    let mut errors: Vec<&String> = got["errors"].as_object().unwrap().keys().collect();
    errors.sort();
    assert_eq!(errors, ["7TV channel", "7TV global"]);
    assert_eq!(got["errors"]["7TV global"], "HTTP 500");
}

#[tokio::test]
async fn at_most_twelve_rooms_are_read_at_once() {
    let rig = Rig::start(&[]).await;
    for i in 0..13 {
        rig.json(&format!("/api/chat/room{i:02}")).await;
    }
    let status = rig.json("/api/status").await;
    let rooms: Vec<&str> = status["chat"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["channel"].as_str().unwrap())
        .collect();
    assert_eq!(rooms.len(), 12);
    assert!(
        !rooms.contains(&"room00"),
        "the least recently read was dropped: {rooms:?}"
    );
    assert!(rooms.contains(&"room12"));
}
