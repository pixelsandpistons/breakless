//! The GraphQL calls the service makes and the reading of their answers: a channel's playback
//! token, which followed channels are live, the most-watched channels. Builds and reads JSON only;
//! the service sends the requests. The queries and the readings are the prototype's
//! (`reference/python/twitchlib.py`: `TOKEN_QUERY`, `token`, `live`, `top`).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

pub use crate::playlist::TOKEN_QUERY;

/// Who is live and what is on, for a list of channel names.
pub const LIVE_QUERY: &str = "query($logins: [String!]) { users(logins: $logins) { login displayName \
                              stream { title viewersCount game { displayName } } } }";

/// The most-watched live channels.
pub const TOP_QUERY: &str = "query($n: Int!) { streams(first: $n) { edges { node { title viewersCount \
                             broadcaster { login displayName } game { displayName } } } } }";

/// A channel name as Twitch spells it in addresses: 2 to 25 letters, digits or underscores.
#[must_use]
pub fn valid_login(name: &str) -> bool {
    (2..=25).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// A request body: the query and its variables.
#[must_use]
pub fn request(query: &str, variables: &Value) -> Value {
    json!({"query": query, "variables": variables})
}

/// The request for a channel's playback token of one kind (`PLAYER_TYPES` lists them).
#[must_use]
pub fn token_request(login: &str, player_type: &str, platform: &str) -> Value {
    request(
        TOKEN_QUERY,
        &json!({"login": login, "playerType": player_type, "platform": platform}),
    )
}

/// The request asking which of `logins` are live, names lowercased and invalid ones dropped.
/// `None` when nothing valid is left to ask about: the prototype makes no request then.
#[must_use]
pub fn live_request(logins: &[String]) -> Option<Value> {
    let logins: Vec<String> = logins
        .iter()
        .filter(|l| valid_login(l))
        .map(|l| l.to_lowercase())
        .collect();
    (!logins.is_empty()).then(|| request(LIVE_QUERY, &json!({"logins": logins})))
}

/// The request for the `n` most-watched channels.
#[must_use]
pub fn top_request(n: u32) -> Value {
    request(TOP_QUERY, &json!({"n": n}))
}

/// Python's truth: `null`, `false`, `0`, `""`, `[]` and `{}` are false, everything else true.
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

/// A string field, or `""` for anything else (the prototype's `s.get("title") or ""`).
fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// The `data` of an answer, or the fault in one line: not JSON, or Twitch's `errors` (the first
/// 300 characters of them, as the prototype reports).
///
/// # Errors
/// `GQL: not JSON: ...` or `GQL: [...]`.
pub fn data(text: &str) -> Result<Value, String> {
    let out: Value = serde_json::from_str(text).map_err(|e| format!("GQL: not JSON: {e}"))?;
    if let Some(errors) = out.get("errors").filter(|e| truthy(e)) {
        return Err(format!(
            "GQL: {}",
            errors.to_string().chars().take(300).collect::<String>()
        ));
    }
    Ok(out
        .get("data")
        .filter(|d| truthy(d))
        .cloned()
        .unwrap_or_else(|| json!({})))
}

/// A channel's playback token: the two halves the master playlist address carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The token itself, a JSON text Twitch signed.
    pub value: String,
    /// Its signature.
    pub signature: String,
}

/// Reads the token from a token request's `data`; `None` when there is none (no such channel).
#[must_use]
pub fn token(data: &Value) -> Option<Token> {
    let t = &data["streamPlaybackAccessToken"];
    Some(Token {
        value: t["value"].as_str()?.to_string(),
        signature: t["signature"].as_str()?.to_string(),
    })
}

/// A followed channel that is live, from the `users` query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Live {
    /// The display name.
    pub name: String,
    /// The stream's title.
    pub title: String,
    /// People watching.
    pub viewers: u64,
    /// The game or category.
    pub game: String,
}

/// Reads `{login: Live}` from a live request's `data`: the users whose `stream` is set.
#[must_use]
pub fn live(data: &Value) -> BTreeMap<String, Live> {
    let mut out = BTreeMap::new();
    for u in data["users"].as_array().into_iter().flatten() {
        let (Some(login), Some(s)) = (u["login"].as_str(), u.get("stream").filter(|s| truthy(s)))
        else {
            continue;
        };
        out.insert(
            login.to_string(),
            Live {
                name: u["displayName"].as_str().unwrap_or(login).to_string(),
                title: text(&s["title"]),
                viewers: s["viewersCount"].as_u64().unwrap_or(0),
                game: text(&s["game"]["displayName"]),
            },
        );
    }
    out
}

/// One of the most-watched channels, from the `streams` query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Top {
    /// The channel's name in addresses.
    pub login: String,
    /// The display name.
    pub name: String,
    /// The stream's title.
    pub title: String,
    /// The game or category.
    pub game: String,
    /// People watching.
    pub viewers: u64,
}

/// Reads the most-watched channels from a top request's `data`, in Twitch's order; an edge
/// without a broadcaster is skipped.
#[must_use]
pub fn top(data: &Value) -> Vec<Top> {
    let mut out = Vec::new();
    for e in data["streams"]["edges"].as_array().into_iter().flatten() {
        let s = &e["node"];
        let Some(login) = s["broadcaster"]["login"].as_str() else {
            continue;
        };
        out.push(Top {
            login: login.to_string(),
            name: s["broadcaster"]["displayName"]
                .as_str()
                .unwrap_or(login)
                .to_string(),
            title: text(&s["title"]),
            game: text(&s["game"]["displayName"]),
            viewers: s["viewersCount"].as_u64().unwrap_or(0),
        });
    }
    out
}
