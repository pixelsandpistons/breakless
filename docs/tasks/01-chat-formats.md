# Task 01: chat parsing and emote readers in `breakless-formats`

Status: open.

## Goal

Port the pure parts of `reference/python/chat.py` to `crates/formats/src/chat.rs` (and
`emotes.rs`), with the 18 tests of `reference/python/test_chat.py` ported one for one, so the service
can later drive a socket through this code without reimplementing any Twitch knowledge.

## Read first

- `reference/python/chat.py`: the module docstring defines the four event shapes. `parse_line`,
  `tag_value`, `parse_emotes`, `event`, `Room.take` (the state machine, without the socket),
  `Room.poll`'s rule for a number from before a restart, `from_bttv`, `from_ffz`, `from_7tv`,
  `SOURCES` (merge order), `image_url`, `emotes` (merge and counts; ignore the threads and cache).
- `reference/python/test_chat.py`: `ParseTests`, `RoomTests` (except the two that sleep on a real
  clock), `EmoteTests`.
- `crates/formats/src/lib.rs` and `playlist.rs` for the house style: documented public items,
  pedantic clippy, no I/O.

## Interface

```rust
// crates/formats/src/chat.rs
pub struct Line { pub tags: BTreeMap<String, String>, pub prefix: String, pub command: String, pub params: Vec<String> }
pub fn parse_line(raw: &str) -> Line;                 // IRCv3 tags unescaped: \s \: \\ \r \n
pub fn parse_emotes(spec: &str) -> Vec<(String, usize, usize)>;   // sorted by start; ids [A-Za-z0-9_]{1,80}; a<=b
#[derive(Serialize)] #[serde(tag = "k")]
pub enum Event {
    #[serde(rename = "m")] Message { id, login, name, color, text, emotes: Vec<(String, usize, usize)>, badges: Vec<String>, me: bool, first: bool, reply: String, bits: u64 },
    #[serde(rename = "n")] Notice { text: String, said: String },
    #[serde(rename = "x")] Clear { login: String },
    #[serde(rename = "d")] Delete { id: String },
}
pub fn event(line: &Line) -> Option<Event>;
pub enum Action { Reply(String), Joined { room_id: String }, Event(Event), Reconnect, Nothing }
pub fn take(line: &Line) -> Action;                   // PING -> Reply("PONG :..."), ROOMSTATE -> Joined, RECONNECT -> Reconnect
pub struct Log { .. }                                 // the last KEEP=200 events with running numbers
impl Log { pub fn add(&mut self, e: Event) -> u64; pub fn since(&self, after: Option<u64>) -> (Vec<&Numbered>, u64 /* seq */); }
//   since(None) and since(Some(n > seq)) give the last 60; since(Some(n)) gives those numbered above n.

// crates/formats/src/emotes.rs
pub type Table = BTreeMap<String, (char, String, bool)>;   // word -> (provider 'b' | 'f' | '7', id, zero_width)
pub fn from_bttv(v: &serde_json::Value) -> Table;
pub fn from_ffz(v: &serde_json::Value) -> Table;
pub fn from_7tv(v: &serde_json::Value) -> Table;
pub const SOURCES: &[Source];                          // (provider, global url, channel url pattern), FrankerFaceZ, BetterTTV, 7TV in that order
pub fn image_url(provider: char, id: &str) -> Option<String>;
pub fn merge(answers: &[(Scope, char, Table)]) -> (Table, Counts);   // globals first, channel wins; later providers win a shared word
```

Serialised `Event::Message` must produce exactly the keys and value shapes the Python emits, in
this order is not required but the names and types are: `k`, `id`, `login`, `name`, `color`, `text`,
`emotes` as arrays of `[id, start, end]`, `badges`, `me`, `first`, `reply`, `bits`. The service adds
`n`. Text is cut to 600 characters, notices to 300; `color` must match `#[0-9A-Fa-f]{6}` or be empty;
badges are filtered to the seven in `BADGES`; a `/me` action shifts emote positions by 8 and strips
the `\x01ACTION ` wrapper. Emote positions count characters, not bytes.

## Dependencies

`serde` with `derive` and `serde_json`, both MIT OR Apache-2.0, added to `crates/formats/Cargo.toml`.
Nothing else.

## Accepted when

- `tools/cargo.sh test -p breakless-formats` passes with the ported tests under
  `crates/formats/tests/chat.rs` and `tests/emotes.rs`, named as sentences.
- A test serialises a message event and compares the JSON object with the one produced by
  `reference/python/chat.py` for the `MSG` line in `test_chat.py` (generate it once with Python,
  paste it as a string in the test, say so in a comment).
- `clippy --all-targets -- -D warnings` and `fmt --check` are clean; every public item is documented.
- Nothing in the crate opens a socket, reads a file or spawns a thread.
