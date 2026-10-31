//! Offline checks of the upstream watcher's command, output and saved-copy handling.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

const VAFT: &str = include_str!("fixtures/vaft-snippet.txt");
const STREAMLINK: &str = include_str!("fixtures/streamlink-snippet.txt");
const NOTES: &str = include_str!("fixtures/notes.txt");
const STATE: &str = include_str!("fixtures/state.json");
const SUMMARY: &str = "vaft 99; 2 advert marks, 3 advert addresses, 4 harmless classes; backup token kinds site, embed\n";
const NOTHING_NEW: &str =
    "nothing new: everything the others look for is accounted for in twitchlib.py\n";
const SOURCE_NAMES: [&str; 5] = [
    "vaft.user.js",
    "streamlink-twitch-plugin.py",
    "ryanbr-issues.md",
    "ryanbr-full-list.md",
    "ttv-lol-pro-README.md",
];

struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "breakless-upstream-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn fixtures(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        for (name, contents) in SOURCE_NAMES
            .into_iter()
            .zip([VAFT, STREAMLINK, NOTES, NOTES, NOTES])
        {
            fs::write(path.join(name), contents).unwrap();
        }
        path
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn run(cwd: &Path, dir: Option<&Path>, environment_dir: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_breakless-tools"));
    command
        .args(["upstream", "--offline"])
        .current_dir(cwd)
        .env_remove("BREAKLESS_UPSTREAM_DIR");
    if let Some(dir) = dir {
        command.arg("--dir").arg(dir);
    }
    if let Some(dir) = environment_dir {
        command.env("BREAKLESS_UPSTREAM_DIR", dir);
    }
    command.output().unwrap()
}

fn stdout(output: &Output, exit: i32) -> String {
    assert_eq!(output.status.code(), Some(exit), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn assert_clean(output: &Output) {
    assert_eq!(stdout(output, 0), format!("{SUMMARY}{NOTHING_NEW}"));
}

#[test]
fn dir_flag_reads_the_selected_fixture_folder_before_the_environment() {
    let folder = Folder::new();
    let selected = folder.fixtures("selected");
    let other = folder.0.join("other");
    assert_clean(&run(&folder.0, Some(&selected), Some(&other)));
    assert!(!selected.join("upstream-state.json").exists());
    assert!(!other.exists());
}

#[test]
fn environment_selects_the_fixture_folder_before_the_default() {
    let folder = Folder::new();
    let selected = folder.fixtures("selected");
    assert_clean(&run(&folder.0, None, Some(&selected)));
    assert!(!folder.0.join("upstream").exists());
}

#[test]
fn default_folder_is_upstream_under_the_current_working_directory() {
    let folder = Folder::new();
    let selected = folder.fixtures("upstream");
    assert_clean(&run(&folder.0, None, None));
    assert!(!selected.join("upstream-state.json").exists());
}

#[test]
fn accounted_facts_leave_an_existing_state_file_untouched_offline() {
    let folder = Folder::new();
    let selected = folder.fixtures("selected");
    let state = selected.join("upstream-state.json");
    fs::write(&state, STATE).unwrap();
    assert_clean(&run(&folder.0, Some(&selected), None));
    assert_eq!(fs::read_to_string(state).unwrap(), STATE);
}

#[test]
fn new_marks_clients_and_token_kinds_report_exact_lines_and_exit_one() {
    let folder = Folder::new();
    let selected = folder.fixtures("selected");
    fs::write(
        selected.join("vaft.user.js"),
        VAFT.replace("'stitched-ad'", "'stitched-ad', 'twitch-brand-new'")
            .replace("['site', 'embed']", "['site', 'embed', 'unknown-player']")
            .replace("kimne78kx3ncx6brgo4mv6wki5h1ko", "other"),
    )
    .unwrap();
    assert_eq!(
        stdout(&run(&folder.0, Some(&selected), None), 1),
        concat!(
            "vaft 99; 3 advert marks, 3 advert addresses, 4 harmless classes; backup token kinds site, embed, unknown-player\n",
            "NEW  vaft advert marks: 'twitch-brand-new' is not accounted for in twitchlib.py\n",
            "NEW  vaft client id is other, ours is kimne78kx3ncx6brgo4mv6wki5h1ko\n",
            "NEW  vaft falls back to token kind 'unknown-player', which probe.py does not measure (PLAYER_TYPES)\n"
        )
    );
}

#[test]
fn a_missing_block_reports_gone_and_exits_one() {
    let folder = Folder::new();
    let selected = folder.fixtures("selected");
    fs::write(
        selected.join("vaft.user.js"),
        VAFT.replace("scope.AdSignifiers", "scope.RenamedSignifiers"),
    )
    .unwrap();
    assert_eq!(
        stdout(&run(&folder.0, Some(&selected), None), 1),
        concat!(
            "vaft 99; 0 advert marks, 3 advert addresses, 4 harmless classes; backup token kinds site, embed\n",
            "GONE vaft advert marks: not found where it used to be; the source was reshaped, read it and fix upstream.py\n"
        )
    );
}

#[test]
fn changed_files_and_facts_keep_the_prototypes_order_and_json_format() {
    let folder = Folder::new();
    let selected = folder.fixtures("selected");
    let mut state: Value = serde_json::from_str(STATE).unwrap();
    for name in SOURCE_NAMES {
        state["files"][name] = json!("old");
    }
    state["facts"]["vaft version"] = json!("98");
    state["facts"]["vaft advert marks"] = json!(["stitched-ad", "é😀"]);
    state["facts"]["streamlink advert marks"] = Value::Null;
    let saved = serde_json::to_vec(&state).unwrap();
    fs::write(selected.join("upstream-state.json"), &saved).unwrap();
    let mut expected = SUMMARY.to_string();
    for name in SOURCE_NAMES {
        writeln!(expected,
            "changed since the last look: research/{name}  (git diff -- twitch/research/{name} shows what)"
        ).unwrap();
    }
    expected.push_str(concat!(
        "was: vaft version = \"98\"\nnow: vaft version = \"99\"\n",
        "was: vaft advert marks = [\"stitched-ad\", \"\\u00e9\\ud83d\\ude00\"]\n",
        "now: vaft advert marks = [\"stitched-ad\", \"twitch-stitched\"]\n",
        "was: streamlink advert marks = null\n",
        "now: streamlink advert marks = [\"streamlink:class:twitch-stitched-ad\", \"streamlink:id:stitched-ad-\", \"streamlink:title:Amazon\"]\n"
    ));
    expected.push_str(NOTHING_NEW);
    assert_eq!(stdout(&run(&folder.0, Some(&selected), None), 0), expected);
    assert_eq!(
        fs::read(selected.join("upstream-state.json")).unwrap(),
        saved
    );
}

#[test]
fn copy_digests_use_decoded_text_and_universal_newlines() {
    let folder = Folder::new();
    let selected = folder.fixtures("selected");
    fs::write(selected.join("vaft.user.js"), VAFT.replace('\n', "\r\n")).unwrap();
    fs::write(
        selected.join("streamlink-twitch-plugin.py"),
        STREAMLINK.replace('\n', "\r"),
    )
    .unwrap();
    fs::write(selected.join("ryanbr-issues.md"), b"invalid:\xff\r\n").unwrap();
    let mut state: Value = serde_json::from_str(STATE).unwrap();
    state["files"]["ryanbr-issues.md"] = json!("9b47c8bf9c6696b5");
    fs::write(
        selected.join("upstream-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    assert_clean(&run(&folder.0, Some(&selected), None));
}

#[test]
fn invalid_state_json_is_ignored_without_being_replaced_offline() {
    let folder = Folder::new();
    let selected = folder.fixtures("selected");
    let state = selected.join("upstream-state.json");
    fs::write(&state, b"{").unwrap();
    assert_clean(&run(&folder.0, Some(&selected), None));
    assert_eq!(fs::read(state).unwrap(), b"{");
}
