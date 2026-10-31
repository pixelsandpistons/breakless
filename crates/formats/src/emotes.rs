//! The emote lists of `BetterTTV`, `FrankerFaceZ` and `7TV`: how each provider's answer reads, where
//! the pictures are, and how the lists merge. No fetching here.
//!
//! Each provider publishes, without a key, a global list and a list per channel (by the channel's
//! Twitch number, which chat states when it is joined). The merged table is
//! `{word: [provider, id, zero_width]}`; a page swaps the word for the picture, which whoever
//! shows it fetches from [`image_url`].

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;
use serde_json::Value;

/// `word -> (provider letter, id, drawn over the emote before it)`.
pub type Table = BTreeMap<String, (char, String, bool)>;

/// Seconds a global list is kept before it is asked for again.
pub const GLOBAL_AGE: u64 = 3600;
/// Seconds a channel's list is kept.
pub const CHANNEL_AGE: u64 = 600;
/// Seconds a failed answer is kept before the provider is asked again.
pub const RETRY_AGE: u64 = 60;

/// A global list or a channel's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// The provider's list for every channel.
    Global,
    /// One channel's list.
    Channel,
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Global => "global",
            Self::Channel => "channel",
        })
    }
}

/// One provider: its letter, how it is named to people, and where its two lists are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// `b`, `f` or `7`.
    pub provider: char,
    /// `BetterTTV`, `FrankerFaceZ`, `7TV`.
    pub name: &'static str,
    /// The global list.
    pub global: &'static str,
    /// A channel's list; `{room_id}` is replaced by the channel's Twitch number.
    pub channel: &'static str,
}

impl Source {
    /// Reads this provider's answer.
    pub fn read(&self, answer: &Value) -> Table {
        match self.provider {
            'b' => from_bttv(answer),
            'f' => from_ffz(answer),
            _ => from_7tv(answer),
        }
    }

    /// The address of one of the two lists.
    pub fn url(&self, scope: Scope, room_id: &str) -> String {
        match scope {
            Scope::Global => self.global.to_string(),
            Scope::Channel => self.channel.replace("{room_id}", room_id),
        }
    }
}

/// The providers in merge order: a later one wins a word two of them both have.
pub const SOURCES: &[Source] = &[
    Source {
        provider: 'f',
        name: "FrankerFaceZ",
        global: "https://api.frankerfacez.com/v1/set/global",
        channel: "https://api.frankerfacez.com/v1/room/id/{room_id}",
    },
    Source {
        provider: 'b',
        name: "BetterTTV",
        global: "https://api.betterttv.net/3/cached/emotes/global",
        channel: "https://api.betterttv.net/3/cached/users/twitch/{room_id}",
    },
    Source {
        provider: '7',
        name: "7TV",
        global: "https://7tv.io/v3/emote-sets/global",
        channel: "https://7tv.io/v3/users/twitch/{room_id}",
    },
];

/// The provider's name to people, for the `errors` keys (`"7TV global"`).
pub fn provider_name(provider: char) -> &'static str {
    SOURCES
        .iter()
        .find(|s| s.provider == provider)
        .map_or("", |s| s.name)
}

fn valid_id(id: &str) -> bool {
    (1..=80).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Where an emote's picture is, at twice the size chat shows it (sharp on a dense screen), or
/// `None` for a provider or an id that is not one. `t` is Twitch's own.
pub fn image_url(provider: char, id: &str) -> Option<String> {
    if !valid_id(id) {
        return None;
    }
    Some(match provider {
        't' => format!("https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/2.0"),
        'b' => format!("https://cdn.betterttv.net/emote/{id}/2x.webp"),
        'f' => format!("https://cdn.frankerfacez.com/emote/{id}/2"),
        '7' => format!("https://cdn.7tv.app/emote/{id}/2x.webp"),
        _ => return None,
    })
}

/// A JSON value as text the way the prototype's `str()` reads it: a string as is, a number as
/// its digits; anything else empty, which no id check accepts.
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// Python's truth of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn rows(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

/// `BetterTTV`'s answer (a list, or `{channelEmotes, sharedEmotes}`).
pub fn from_bttv(data: &Value) -> Table {
    let mut out = Table::new();
    let mut add = |e: &Value| {
        if !e.is_object() {
            return;
        }
        let code = e["code"].as_str().unwrap_or("");
        let id = text(&e["id"]);
        if !code.is_empty() && image_url('b', &id).is_some() {
            out.insert(code.to_string(), ('b', id, truthy(&e["modifier"])));
        }
    };
    if data.is_array() {
        rows(data).for_each(&mut add);
    } else {
        rows(&data["channelEmotes"])
            .chain(rows(&data["sharedEmotes"]))
            .for_each(&mut add);
    }
    out
}

/// `FrankerFaceZ`'s answer (`{sets: {id: {emoticons}}}`; the global one limited to `default_sets`).
pub fn from_ffz(data: &Value) -> Table {
    let empty = serde_json::Map::new();
    let sets = data["sets"].as_object().unwrap_or(&empty);
    let wanted: Vec<String> = match data.get("default_sets") {
        Some(d) => rows(d).map(text).collect(),
        None => sets.keys().cloned().collect(),
    };
    let mut out = Table::new();
    for sid in wanted {
        let Some(set) = sets.get(&sid) else {
            continue;
        };
        for e in rows(&set["emoticons"]) {
            let name = e["name"].as_str().unwrap_or("");
            let id = text(&e["id"]);
            if !name.is_empty() && image_url('f', &id).is_some() {
                out.insert(name.to_string(), ('f', id, truthy(&e["modifier"])));
            }
        }
    }
    out
}

/// `7TV`'s answer (a set, or a user with `emote_set`). Bit 0 of `flags` on an emote means it is
/// drawn over the one before.
pub fn from_7tv(data: &Value) -> Table {
    let list = if data.get("emote_set").is_some() {
        &data["emote_set"]["emotes"]
    } else {
        &data["emotes"]
    };
    let mut out = Table::new();
    for e in rows(list) {
        if !e.is_object() {
            continue;
        }
        let name = e["name"].as_str().unwrap_or("");
        let id = text(&e["id"]);
        if !name.is_empty() && image_url('7', &id).is_some() {
            let flags = match &e["flags"] {
                Value::Number(n) => n.as_u64().unwrap_or(0),
                Value::String(s) => s.trim().parse().unwrap_or(0),
                _ => 0,
            };
            out.insert(name.to_string(), ('7', id, flags & 1 == 1));
        }
    }
    out
}

/// How many words each list gave, by provider letter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Count {
    /// From the global list.
    pub global: usize,
    /// From the channel's list.
    pub channel: usize,
}

/// Counts by provider letter.
pub type Counts = BTreeMap<char, Count>;

/// Merges answers given in order: global lists first and then the channel's, so a channel's word
/// wins; within a scope a later provider wins a shared word. Every provider that answered gets a
/// count for both scopes, zero where it gave nothing.
pub fn merge(answers: &[(Scope, char, Table)]) -> (Table, Counts) {
    let mut out = Table::new();
    let mut counts = Counts::new();
    for (scope, provider, table) in answers {
        out.extend(table.iter().map(|(k, v)| (k.clone(), v.clone())));
        let c = counts.entry(*provider).or_default();
        match scope {
            Scope::Global => c.global = table.len(),
            Scope::Channel => c.channel = table.len(),
        }
    }
    (out, counts)
}

/// A channel number as Twitch gives one: up to 20 digits.
pub fn valid_room_id(room_id: &str) -> bool {
    (1..=20).contains(&room_id.len()) && room_id.bytes().all(|b| b.is_ascii_digit())
}
