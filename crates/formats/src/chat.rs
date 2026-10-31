//! Twitch chat as IRC: lines, tags, the events a reader sees, and the room's rules. No sockets.
//!
//! Anyone may read a channel's chat by joining as `justinfanNNNN` with no password; such a guest
//! cannot write. The service keeps one connection per channel being read and turns each line into
//! an [`Event`] with a running number ([`Log`]); a page asks for everything after number N.
//!
//! Emote positions in a message count characters, not bytes, as Twitch sends them.

use std::collections::{BTreeMap, VecDeque};

use serde::Serialize;

/// Events remembered per channel.
pub const KEEP: usize = 200;
/// Events handed to a reader who names no number, or a number from before a restart.
pub const RECENT: usize = 60;
/// Badges worth showing as a word beside a name; the rest (bits, sub-gifter, hype train...) are
/// left out.
pub const BADGES: &[&str] = &[
    "broadcaster",
    "moderator",
    "vip",
    "staff",
    "partner",
    "subscriber",
    "founder",
];
const TEXT_MAX: usize = 600;
const NOTICE_MAX: usize = 300;

/// One IRC line, split. `params`' last item is the trailing text when there was one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Line {
    /// `IRCv3` tags, values unescaped.
    pub tags: BTreeMap<String, String>,
    /// The sender (`login!login@login.tmi.twitch.tv`), or the server.
    pub prefix: String,
    /// `PRIVMSG`, `PING`, `ROOMSTATE`...
    pub command: String,
    /// The arguments; the trailing text, if any, last.
    pub params: Vec<String>,
}

/// An `IRCv3` tag value with `\s`, `\:`, `\\`, `\r` and `\n` unescaped. A lone backslash before
/// any other character is dropped, as Twitch's own clients do.
pub fn tag_value(v: &str) -> String {
    if !v.contains('\\') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some(':') => out.push(';'),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// One raw line as a [`Line`].
pub fn parse_line(raw: &str) -> Line {
    let mut line = raw;
    let mut tags = BTreeMap::new();
    if let Some(rest) = line.strip_prefix('@') {
        let (tag_text, after) = rest.split_once(' ').unwrap_or((rest, ""));
        for pair in tag_text.split(';') {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            tags.insert(k.to_string(), tag_value(v));
        }
        line = after;
    }
    let mut prefix = String::new();
    if let Some(rest) = line.strip_prefix(':') {
        let (p, after) = rest.split_once(' ').unwrap_or((rest, ""));
        prefix = p.to_string();
        line = after;
    }
    let (head, trailing) = match line.split_once(" :") {
        Some((h, t)) => (h, Some(t)),
        None => (line, None),
    };
    let mut params: Vec<String> = head.split_whitespace().map(str::to_string).collect();
    let command = if params.is_empty() {
        String::new()
    } else {
        params.remove(0)
    };
    if let Some(t) = trailing {
        params.push(t.to_string());
    }
    Line {
        tags,
        prefix,
        command,
        params,
    }
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn valid_id(id: &str) -> bool {
    (1..=80).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// `"25:0-4,12-16/1902:6-10"` as `(id, first character, last character)`, sorted by position.
/// A range whose end comes before its start, or an id that is not one, is dropped.
pub fn parse_emotes(spec: &str) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    for part in spec.split('/') {
        let (eid, ranges) = part.split_once(':').unwrap_or((part, ""));
        if !valid_id(eid) {
            continue;
        }
        for r in ranges.split(',') {
            let Some((a, b)) = r.split_once('-') else {
                continue;
            };
            if !digits(a) || !digits(b) {
                continue;
            }
            let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) else {
                continue;
            };
            if a <= b {
                out.push((eid.to_string(), a, b));
            }
        }
    }
    out.sort_by_key(|e| e.1);
    out
}

/// What a reader sees, as the page's JSON: `k` says which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "k")]
pub enum Event {
    /// Somebody wrote.
    #[serde(rename = "m")]
    Message {
        /// Twitch's message id.
        id: String,
        /// The writer's name in addresses.
        login: String,
        /// The writer's display name.
        name: String,
        /// `#RRGGBB`, or empty.
        color: String,
        /// The text, at most 600 characters.
        text: String,
        /// Twitch's own emotes as `[id, first character, last character]`.
        emotes: Vec<(String, usize, usize)>,
        /// Those of [`BADGES`] the writer carries.
        badges: Vec<String>,
        /// A `/me` action.
        me: bool,
        /// The writer's first message in this channel.
        first: bool,
        /// The display name of who is being replied to, or empty.
        reply: String,
        /// Bits cheered with the message.
        bits: u64,
    },
    /// A notice of the channel: a subscription, a raid.
    #[serde(rename = "n")]
    Notice {
        /// What happened, in Twitch's words.
        text: String,
        /// What the person said with it, or empty.
        said: String,
    },
    /// That person's messages were removed (`""`: the whole chat was cleared).
    #[serde(rename = "x")]
    Clear {
        /// Whose, or empty for everyone's.
        login: String,
    },
    /// One message was removed.
    #[serde(rename = "d")]
    Delete {
        /// Which one.
        id: String,
    },
}

fn cut(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

fn is_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit())
}

/// A line as the event a reader sees, or `None` for a line that is not one.
pub fn event(line: &Line) -> Option<Event> {
    let tag = |k: &str| line.tags.get(k).map_or("", String::as_str);
    match line.command.as_str() {
        "PRIVMSG" if line.params.len() >= 2 => {
            let raw = line.params[1].as_str();
            let (text, me, shift) = match raw
                .strip_prefix("\u{1}ACTION ")
                .and_then(|t| t.strip_suffix('\u{1}'))
            {
                Some(t) => (t, true, 8),
                None => (raw, false, 0),
            };
            let login = line.prefix.split('!').next().unwrap_or("").to_string();
            let name = match tag("display-name") {
                "" => login.clone(),
                n => n.to_string(),
            };
            let color = tag("color");
            Some(Event::Message {
                id: tag("id").to_string(),
                name,
                login,
                color: if is_colour(color) {
                    color.to_string()
                } else {
                    String::new()
                },
                text: cut(text, TEXT_MAX),
                emotes: parse_emotes(tag("emotes"))
                    .into_iter()
                    .map(|(i, a, b)| (i, a.saturating_sub(shift), b.saturating_sub(shift)))
                    .collect(),
                badges: tag("badges")
                    .split(',')
                    .map(|b| b.split('/').next().unwrap_or(""))
                    .filter(|b| BADGES.contains(b))
                    .map(str::to_string)
                    .collect(),
                me,
                first: tag("first-msg") == "1",
                reply: tag("reply-parent-display-name").to_string(),
                bits: {
                    let b = tag("bits");
                    if !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()) {
                        b.parse().unwrap_or(u64::MAX)
                    } else {
                        0
                    }
                },
            })
        }
        "USERNOTICE" => {
            let text = tag("system-msg");
            if text.is_empty() {
                return None;
            }
            Some(Event::Notice {
                text: cut(text, NOTICE_MAX),
                said: cut(line.params.get(1).map_or("", String::as_str), NOTICE_MAX),
            })
        }
        "CLEARCHAT" => Some(Event::Clear {
            login: line.params.get(1).cloned().unwrap_or_default(),
        }),
        "CLEARMSG" => Some(Event::Delete {
            id: tag("target-msg-id").to_string(),
        }),
        _ => None,
    }
}

/// What one parsed line does to a room.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Send this line back (a `PONG`).
    Reply(String),
    /// Twitch confirmed the join and stated the channel's number.
    Joined {
        /// The channel's Twitch number, which the emote providers key their lists by.
        room_id: String,
    },
    /// Something for the readers.
    Event(Event),
    /// Twitch asked for a reconnect: drop the connection and open a new one.
    Reconnect,
    /// Nothing to do.
    Nothing,
}

/// The room's rule for one line.
pub fn take(line: &Line) -> Action {
    match line.command.as_str() {
        "PING" => Action::Reply(format!(
            "PONG :{}",
            line.params.last().map_or("tmi.twitch.tv", String::as_str)
        )),
        "ROOMSTATE" if line.tags.get("room-id").is_some_and(|id| !id.is_empty()) => {
            Action::Joined {
                room_id: line.tags["room-id"].clone(),
            }
        }
        "RECONNECT" => Action::Reconnect,
        _ => event(line).map_or(Action::Nothing, Action::Event),
    }
}

/// An event with its running number, as the page receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Numbered {
    /// The running number, from 1.
    pub n: u64,
    /// The event.
    #[serde(flatten)]
    pub event: Event,
}

/// The last [`KEEP`] events of one room, numbered from 1.
#[derive(Debug, Default)]
pub struct Log {
    events: VecDeque<Numbered>,
    seq: u64,
}

impl Log {
    /// Adds an event and returns its number.
    pub fn add(&mut self, event: Event) -> u64 {
        self.seq += 1;
        if self.events.len() == KEEP {
            self.events.pop_front();
        }
        self.events.push_back(Numbered { n: self.seq, event });
        self.seq
    }

    /// The number of the last event, 0 before any.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// How many events are kept right now.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Nothing has been said.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The events numbered above `after`. Without a number, or with one above the last given
    /// (a reader from before a restart), the last [`RECENT`].
    pub fn since(&self, after: Option<u64>) -> Vec<&Numbered> {
        match after {
            Some(n) if n <= self.seq => self.events.iter().filter(|e| e.n > n).collect(),
            _ => self
                .events
                .iter()
                .skip(self.events.len().saturating_sub(RECENT))
                .collect(),
        }
    }
}
