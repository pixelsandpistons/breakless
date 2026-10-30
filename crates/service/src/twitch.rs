//! Talking to Twitch without an account: a token, the playlists, who is live. Through an exit or
//! straight out. The formats are `breakless_formats`'; this module only moves bytes.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use breakless_formats::{CLIENT_ID, TOKEN_QUERY};
use serde_json::{Value, json};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const TIMEOUT: Duration = Duration::from_secs(10);
/// Seconds an exit is left alone after it failed.
pub const EXIT_REST: Duration = Duration::from_secs(30);

/// A failed request. `code` is the HTTP status, or 0 when nothing came back.
#[derive(Debug, Clone)]
pub struct Error {
    /// The HTTP status, or 0 when nothing came back.
    pub code: u16,
    /// What went wrong, in one line.
    pub message: String,
}

impl Error {
    fn new(code: u16, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// A dropped connection, a timeout or a gateway status: worth one more try.
    pub fn is_passing(&self) -> bool {
        matches!(self.code, 0 | 502 | 503 | 504)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A random 32-character lowercase id, as the web player makes for `X-Device-Id`.
pub fn device_id() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(32);
    let mut bits = random_u64();
    for i in 0..32 {
        if i % 10 == 0 {
            bits = random_u64();
        }
        out.push(ALPHABET[(bits % ALPHABET.len() as u64) as usize] as char);
        bits /= ALPHABET.len() as u64;
    }
    out
}

/// A random number from the standard library's randomly seeded hasher. Not for secrets.
pub fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::hash::RandomState::new().build_hasher().finish()
}

/// A POST's body and extra headers.
pub type PostBody<'a> = (Vec<u8>, Vec<(&'a str, String)>);

/// Makes ring the process's TLS provider. reqwest is built without one of its own (ring needs no
/// C toolchain beyond a compiler); every binary that builds a `reqwest::Client` calls this first.
/// Calling it twice is harmless.
pub fn install_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// One way out: straight, or through an HTTP proxy.
#[derive(Clone)]
pub struct Via {
    client: reqwest::Client,
}

impl Via {
    fn build(proxy: Option<&str>) -> Result<Self, reqwest::Error> {
        install_tls();
        let mut b = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(TIMEOUT)
            .no_proxy();
        if let Some(url) = proxy {
            b = b.proxy(reqwest::Proxy::all(url)?);
        }
        Ok(Self { client: b.build()? })
    }
}

#[derive(Default)]
struct ExitState {
    down_until: Option<Instant>,
    last_error: String,
    last_ok: Option<Instant>,
    country: String,
    cluster: String,
}

/// A proxy in a country without adverts, and whether it is answering.
pub struct Exit {
    /// The proxy's host, for logs and status. Never the whole address: it may carry a password.
    pub label: String,
    via: Via,
    state: Mutex<ExitState>,
}

impl Exit {
    fn new(url: &str) -> Result<Self, reqwest::Error> {
        let label = url.split("://").nth(1).unwrap_or(url);
        let label = label
            .rsplit('@')
            .next()
            .unwrap_or(label)
            .trim_end_matches('/')
            .to_string();
        Ok(Self {
            label,
            via: Via::build(Some(url))?,
            state: Mutex::new(ExitState::default()),
        })
    }

    /// `true` unless it failed within the last [`EXIT_REST`].
    pub fn usable(&self) -> bool {
        let s = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.down_until.is_none_or(|t| Instant::now() >= t)
    }

    /// Records a failure: the exit rests for [`EXIT_REST`].
    pub fn failed(&self, why: &str) {
        let mut s = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.down_until = Some(Instant::now() + EXIT_REST);
        s.last_error = format!("{} {}", now_text(), truncate(why, 120));
        tracing::warn!(
            "exit {} failed, using the home line for {}s: {}",
            self.label,
            EXIT_REST.as_secs(),
            truncate(why, 120)
        );
    }

    /// Records a master playlist that came through: the country and cluster Twitch stated.
    pub fn ok(&self, country: &str, cluster: &str) {
        let mut s = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        s.last_ok = Some(Instant::now());
        s.country = country.to_string();
        s.cluster = cluster.to_string();
    }

    /// The exit's part of `/api/status`.
    pub fn status(&self) -> Value {
        let s = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        json!({
            "configured": true,
            "proxy": self.label,
            "usable": s.down_until.is_none_or(|t| Instant::now() >= t),
            "country": s.country,
            "cluster": s.cluster,
            "last_ok_seconds_ago": s.last_ok.map(|t| t.elapsed().as_secs()),
            "last_error": s.last_error,
        })
    }
}

/// The client: the home line and the exits.
pub struct Twitch {
    direct: Via,
    /// In the order they are tried.
    pub exits: Vec<Exit>,
    gql_url: String,
    usher_url: String,
}

/// Who is live, from the `users` query.
#[derive(Debug, Clone, serde::Serialize)]
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

/// One of the most-watched channels.
#[derive(Debug, Clone, serde::Serialize)]
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

impl Twitch {
    /// Builds the clients. `proxies` are HTTP proxy addresses, in the order to try them.
    pub fn new(proxies: &[String], gql_url: &str, usher_url: &str) -> Result<Self, reqwest::Error> {
        Ok(Self {
            direct: Via::build(None)?,
            exits: proxies
                .iter()
                .map(|p| Exit::new(p))
                .collect::<Result<_, _>>()?,
            gql_url: gql_url.to_string(),
            usher_url: usher_url.to_string(),
        })
    }

    /// The way out for an exit index, or straight out for `None`.
    pub fn via(&self, exit: Option<usize>) -> &Via {
        exit.and_then(|i| self.exits.get(i))
            .map_or(&self.direct, |e| &e.via)
    }

    /// The first exit that is not resting.
    pub fn first_usable_exit(&self) -> Option<usize> {
        self.exits.iter().position(Exit::usable)
    }

    /// A GET (no body) or POST (a body) returning the text; non-2xx is an [`Error`].
    pub async fn http(
        &self,
        via: &Via,
        url: &str,
        body: Option<PostBody<'_>>,
    ) -> Result<String, Error> {
        let host = reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let request = match body {
            None => via.client.get(url),
            Some((bytes, headers)) => {
                let mut r = via.client.post(url).body(bytes);
                for (k, v) in headers {
                    r = r.header(k, v);
                }
                r
            }
        };
        let response = request
            .send()
            .await
            .map_err(|e| Error::new(0, format!("{host}: {e}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::new(0, format!("{host}: {e}")))?;
        if !status.is_success() {
            return Err(Error::new(
                status.as_u16(),
                format!(
                    "HTTP {} from {host}: {}",
                    status.as_u16(),
                    truncate(&text, 200)
                ),
            ));
        }
        Ok(text)
    }

    async fn gql(&self, via: &Via, query: &str, variables: Value) -> Result<Value, Error> {
        let body = serde_json::to_vec(&json!({"query": query, "variables": variables}))
            .unwrap_or_default();
        let headers = vec![
            ("Client-ID", CLIENT_ID.to_string()),
            ("X-Device-Id", device_id()),
            ("Content-Type", "text/plain;charset=UTF-8".to_string()),
        ];
        let text = self.http(via, &self.gql_url, Some((body, headers))).await?;
        let out: Value = serde_json::from_str(&text)
            .map_err(|e| Error::new(0, format!("GQL: not JSON: {e}")))?;
        if out
            .get("errors")
            .is_some_and(|e| e.as_array().is_some_and(|a| !a.is_empty()))
        {
            return Err(Error::new(
                0,
                format!("GQL: {}", truncate(&out["errors"].to_string(), 300)),
            ));
        }
        Ok(out.get("data").cloned().unwrap_or(Value::Null))
    }

    /// A playback token: `(value, signature)`. `Err` with a message starting `no token` means no
    /// such channel.
    pub async fn token(
        &self,
        via: &Via,
        login: &str,
        player_type: &str,
        platform: &str,
    ) -> Result<(String, String), Error> {
        let data = self
            .gql(
                via,
                TOKEN_QUERY,
                json!({"login": login, "playerType": player_type, "platform": platform}),
            )
            .await?;
        let t = &data["streamPlaybackAccessToken"];
        match (t["value"].as_str(), t["signature"].as_str()) {
            (Some(v), Some(s)) => Ok((v.to_string(), s.to_string())),
            _ => Err(Error::new(
                0,
                format!("no token for {login} as {player_type}"),
            )),
        }
    }

    /// The master playlist address for a token. An unparsable usher address (only tests set
    /// one) gives an address that will fail with a plain error.
    pub fn master_url(&self, login: &str, value: &str, signature: &str) -> String {
        let base = self.usher_url.replace("{login}", &login.to_lowercase());
        let mut url = reqwest::Url::parse(&base)
            .or_else(|_| reqwest::Url::parse("http://invalid.invalid/"))
            .unwrap_or_else(|_| unreachable!("a constant address parses"));
        let p = 1_000_000 + random_u64() % 9_000_000;
        url.query_pairs_mut()
            .append_pair("player", "twitchweb")
            .append_pair("p", &p.to_string())
            .append_pair("type", "any")
            .append_pair("allow_source", "true")
            .append_pair("allow_audio_only", "true")
            .append_pair("allow_spectre", "false")
            .append_pair("playlist_include_framerate", "true")
            .append_pair("supported_codecs", "h264")
            .append_pair("fast_bread", "true")
            .append_pair("sig", signature)
            .append_pair("token", value);
        url.to_string()
    }

    /// The master playlist text for a live channel. `Err` with code 404 when it is offline.
    pub async fn master(&self, via: &Via, login: &str) -> Result<String, Error> {
        let (value, signature) = self.token(via, login, "site", "web").await?;
        self.http(via, &self.master_url(login, &value, &signature), None)
            .await
    }

    /// `{login: Live}` for those of `logins` that are live now. Asked straight out.
    pub async fn live(&self, logins: &[String]) -> Result<BTreeMap<String, Live>, Error> {
        if logins.is_empty() {
            return Ok(BTreeMap::new());
        }
        let q = "query($logins: [String!]) { users(logins: $logins) { login displayName \
                 stream { title viewersCount game { displayName } } } }";
        let data = self.gql(&self.direct, q, json!({"logins": logins})).await?;
        let mut out = BTreeMap::new();
        for u in data["users"].as_array().into_iter().flatten() {
            let (Some(login), Some(s)) = (
                u["login"].as_str(),
                u.get("stream").filter(|s| s.is_object()),
            ) else {
                continue;
            };
            out.insert(
                login.to_string(),
                Live {
                    name: u["displayName"].as_str().unwrap_or(login).to_string(),
                    title: s["title"].as_str().unwrap_or("").to_string(),
                    viewers: s["viewersCount"].as_u64().unwrap_or(0),
                    game: s["game"]["displayName"].as_str().unwrap_or("").to_string(),
                },
            );
        }
        Ok(out)
    }

    /// The `n` most-watched live channels. Asked straight out.
    pub async fn top(&self, n: u32) -> Result<Vec<Top>, Error> {
        let q = "query($n: Int!) { streams(first: $n) { edges { node { title viewersCount \
                 broadcaster { login displayName } game { displayName } } } } }";
        let data = self.gql(&self.direct, q, json!({"n": n})).await?;
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
                title: s["title"].as_str().unwrap_or("").to_string(),
                game: s["game"]["displayName"].as_str().unwrap_or("").to_string(),
                viewers: s["viewersCount"].as_u64().unwrap_or(0),
            });
        }
        Ok(out)
    }
}

/// The first `n` characters.
pub fn truncate(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, for the records people read.
pub fn now_text() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(i64::try_from(days).unwrap_or(0));
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ids_are_32_lowercase_and_differ() {
        let a = device_id();
        let b = device_id();
        assert_eq!(a.len(), 32);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
        assert_ne!(a, b);
    }

    #[test]
    fn the_exit_label_hides_credentials() {
        let e = Exit::new("http://user:secret@exit:8888").unwrap();
        assert_eq!(e.label, "exit:8888");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20730), (2026, 10, 4));
    }

    #[test]
    fn the_master_url_carries_the_token() {
        let t = Twitch::new(&[], "http://g/", "http://u/api/channel/hls/{login}.m3u8").unwrap();
        let url = t.master_url("SomeOne", "tok", "sig");
        assert!(url.starts_with("http://u/api/channel/hls/someone.m3u8?player=twitchweb&p="));
        assert!(url.contains("&sig=sig&token=tok"));
        assert!(url.contains("&supported_codecs=h264&"));
    }
}
