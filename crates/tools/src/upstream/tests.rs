use super::*;

use std::io::Read;
use std::net::TcpListener;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

const VAFT: &str = include_str!("../../tests/fixtures/vaft-snippet.txt");
const STREAMLINK: &str = include_str!("../../tests/fixtures/streamlink-snippet.txt");
const NOTES: &str = include_str!("../../tests/fixtures/notes.txt");
const STATE: &str = include_str!("../../tests/fixtures/state.json");

struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "breakless-upstream-unit-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn serve(status: &str, body: &[u8]) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/source", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let body = body.to_vec();
    let worker = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            request.extend(byte);
        }
        socket.write_all(response.as_bytes()).unwrap();
        socket.write_all(&body).unwrap();
    });
    (url, worker)
}

fn fixture_facts() -> Facts {
    facts(&BTreeMap::from([
        ("vaft.user.js".to_string(), VAFT.to_string()),
        (
            "streamlink-twitch-plugin.py".to_string(),
            STREAMLINK.to_string(),
        ),
    ]))
}

#[test]
fn javascript_lists_keep_escapes_empty_strings_and_the_first_block() {
    let text = r#"scope.AdSignifiers = ['a\'b', "c\"d", '', // 'ignored'
        'kept']; scope.AdSignifiers = ['second'];"#;
    assert_eq!(
        js_list(text, "AdSignifiers"),
        json!([r"a\'b", r#"c\"d"#, "", "kept"])
    );
    assert_eq!(js_list(text, "Missing"), Value::Null);
    assert_eq!(js_list("scope.Empty = [];", "Empty"), json!([]));
}

#[test]
fn streamlink_marks_group_classes_then_ids_then_titles() {
    let text = concat!(
        "def _is_daterange_ad(value):\n",
        "    startswith(\"first-id\") or classname == \"first-class\"\n",
        "    classname == \"second-class\" or startswith(\"second-id\")\n\n",
        "def _is_segment_ad(title):\n",
        "    return \"first-title\" in title or \"second-title\" in title\n\n"
    );
    assert_eq!(
        streamlink_marks(text),
        json!([
            "streamlink:class:first-class",
            "streamlink:class:second-class",
            "streamlink:id:first-id",
            "streamlink:id:second-id",
            "streamlink:title:first-title",
            "streamlink:title:second-title"
        ])
    );
    for missing in ["_is_daterange_ad", "_is_segment_ad"] {
        assert_eq!(
            streamlink_marks(&text.replace(missing, "renamed")),
            Value::Null
        );
    }
}

#[test]
fn accounted_fixture_facts_include_both_clients_and_need_no_action() {
    let found = fixture_facts();
    assert_eq!(value(&found, "vaft version"), "99");
    assert_eq!(value(&found, "vaft client id"), CLIENT_ID);
    assert_eq!(value(&found, "streamlink client id"), CLIENT_ID);
    assert_eq!(compare(&found), [] as [(&str, String); 0]);
}

#[test]
fn missing_facts_precede_new_marks_clients_and_token_kinds() {
    let mut found = fixture_facts();
    for (name, value) in &mut found {
        *value = match *name {
            "vaft version" => Value::Null,
            "vaft advert marks" => json!(["stitched-ad", "twitch-new"]),
            "vaft client id" | "streamlink client id" => json!("other"),
            "vaft backup token kinds" => json!(["site", "unknown"]),
            _ => value.clone(),
        };
    }
    let problems = compare(&found);
    assert_eq!(
        problems.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        ["GONE", "NEW", "NEW", "NEW", "NEW"]
    );
    assert!(problems[1].1.contains("'twitch-new'"));
    assert!(problems[2].1.starts_with("vaft client id is other"));
    assert!(problems[3].1.starts_with("streamlink client id is other"));
    assert!(problems[4].1.contains("token kind 'unknown'"));
}

#[tokio::test]
async fn a_499_byte_download_preserves_the_copy_and_500_bytes_replace_it() {
    breakless::twitch::install_tls();
    let folder = Folder::new();
    let path = folder.0.join("copy");
    std::fs::write(&path, b"saved").unwrap();
    let client = reqwest::Client::new();
    let (url, worker) = serve("200 OK", &vec![b'x'; 499]);
    assert_eq!(
        refresh(&client, &url, &path).await.unwrap_err().to_string(),
        "only 499 bytes"
    );
    worker.join().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"saved");
    let body = vec![b'y'; 500];
    let (url, worker) = serve("200 OK", &body);
    refresh(&client, &url, &path).await.unwrap();
    worker.join().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
}

#[tokio::test]
async fn an_http_error_preserves_the_saved_copy() {
    breakless::twitch::install_tls();
    let folder = Folder::new();
    let path = folder.0.join("copy");
    std::fs::write(&path, b"saved").unwrap();
    let (url, worker) = serve("503 Service Unavailable", &[]);
    let error = refresh(&reqwest::Client::new(), &url, &path)
        .await
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.to_string(), "HTTP Error 503: Service Unavailable");
    assert_eq!(std::fs::read(path).unwrap(), b"saved");
}

#[test]
fn state_json_uses_sorted_keys_ascii_escapes_and_one_space_indentation() {
    assert_eq!(
        state_json(&json!({"z": "é😀", "a": [true, null, {"x": "v"}]})).unwrap(),
        concat!(
            "{\n \"a\": [\n  true,\n  null,\n  {\n   \"x\": \"v\"\n  }\n ],\n",
            " \"z\": \"\\u00e9\\ud83d\\ude00\"\n}"
        )
    );
}

#[tokio::test]
async fn network_failures_use_saved_copies_and_online_state_keeps_metadata() {
    breakless::twitch::install_tls();
    let folder = Folder::new();
    for ((name, _), text) in SOURCES.iter().zip([VAFT, STREAMLINK, NOTES, NOTES, NOTES]) {
        std::fs::write(folder.0.join(name), text).unwrap();
    }
    let mut expected: Value = serde_json::from_str(STATE).unwrap();
    expected["metadata"] = json!({"label": "é😀"});
    let state_path = folder.0.join("upstream-state.json");
    std::fs::write(&state_path, serde_json::to_vec(&expected).unwrap()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/source", listener.local_addr().unwrap());
    drop(listener);
    let sources: Vec<_> = SOURCES
        .iter()
        .map(|(name, _)| (*name, url.as_str()))
        .collect();
    let args = Args {
        offline: false,
        dir: folder.0.clone(),
    };
    let before = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M").to_string();
    let mut output = Vec::new();
    assert_eq!(inspect(&args, &sources, &mut output).await.unwrap(), 0);
    let after = jiff::Zoned::now().strftime("%Y-%m-%d %H:%M").to_string();
    let output = String::from_utf8(output).unwrap();
    assert_eq!(output.matches("could not fetch ").count(), 5);
    assert_eq!(output.matches("(using the copy in upstream/)").count(), 5);
    assert!(output.ends_with(
        "nothing new: everything the others look for is accounted for in twitchlib.py\n"
    ));
    let saved = std::fs::read_to_string(state_path).unwrap();
    let state: Value = serde_json::from_str(&saved).unwrap();
    assert!([before.as_str(), after.as_str()].contains(&state["looked"].as_str().unwrap()));
    expected["looked"] = state["looked"].clone();
    assert_eq!(state, expected);
    assert_eq!(saved, state_json(&expected).unwrap());
    assert!(saved.is_ascii());
    assert!(!saved.ends_with('\n'));
}

#[test]
fn new_mark_strings_keep_pythons_quotes_and_nonprintable_escapes() {
    assert_eq!(python_repr("a'b"), r#""a'b""#);
    assert_eq!(python_repr("a'\"b"), r#"'a\'"b'"#);
    assert_eq!(
        python_repr("é\u{a0}\u{200b}\n\t\r\\\0\u{7f}"),
        r"'é\xa0\u200b\n\t\r\\\x00\x7f'"
    );
    assert_eq!(python_repr("\u{e0001}"), r"'\U000e0001'");
}

#[test]
fn changed_fact_json_keeps_pythons_spacing_and_unicode_escapes() {
    assert_eq!(
        python_json(&json!({"x": ["é😀", {"y": []}], "z": "\u{7f}"})),
        r#"{"x": ["\u00e9\ud83d\ude00", {"y": []}], "z": "\u007f"}"#
    );
}
