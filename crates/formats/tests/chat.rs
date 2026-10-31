#![allow(clippy::assert_is_empty)] // `assert!(x.is_empty())` reads as the sentence it proves

//! Chat parsing and the room's rules, one test per case of the prototype's `test_chat.py`
//! (`ParseTests` and `RoomTests`, except the two that wait on a real clock: those are the
//! service's).

use breakless_formats::chat::{
    Action, Event, KEEP, Log, RECENT, event, parse_emotes, parse_line, take,
};
use serde_json::{Value, json};

const MSG: &str = "@badge-info=subscriber/14;badges=moderator/1,subscriber/12,bits/1000;color=#1E90FF;display-name=Some\\sOne;\
emotes=25:6-10,18-22/1902:12-16;first-msg=0;id=abc-1;room-id=71092938;tmi-sent-ts=1;user-id=7 \
:someone!someone@someone.tmi.twitch.tv PRIVMSG #chan :hello Kappa Keepo Kappa: done";

fn message(line: &str) -> Event {
    event(&parse_line(line)).expect("a message event")
}

#[test]
fn tags_are_unescaped() {
    let l = parse_line(MSG);
    assert_eq!(l.tags["display-name"], "Some One");
    assert_eq!(l.prefix.split('!').next(), Some("someone"));
    assert_eq!(l.command, "PRIVMSG");
    assert_eq!(l.params[0], "#chan");
    assert_eq!(l.params[1], "hello Kappa Keepo Kappa: done"); // a colon inside the text stays
}

#[test]
fn message_event() {
    let Event::Message {
        login,
        name,
        color,
        id,
        emotes,
        badges,
        text,
        ..
    } = message(MSG)
    else {
        panic!("not a message")
    };
    assert_eq!(
        (login.as_str(), name.as_str(), color.as_str(), id.as_str()),
        ("someone", "Some One", "#1E90FF", "abc-1")
    );
    assert_eq!(
        emotes,
        [
            ("25".to_string(), 6, 10),
            ("1902".to_string(), 12, 16),
            ("25".to_string(), 18, 22)
        ]
    );
    assert_eq!(badges, ["moderator", "subscriber"]); // bits is not a badge we show
    let words: Vec<String> = emotes
        .iter()
        .map(|(_, a, b)| text.chars().skip(*a).take(b - a + 1).collect())
        .collect();
    assert_eq!(words, ["Kappa", "Keepo", "Kappa"]);
}

#[test]
fn the_message_event_serialises_as_the_prototype_does() {
    // Produced once by `reference/python/chat.py` for MSG:
    //   python3 -c 'import json,sys; sys.path.insert(0,"reference/python"); import chat,test_chat;
    //   print(json.dumps(chat.event(*chat.parse_line(test_chat.MSG)), sort_keys=True))'
    let python = r##"{"badges": ["moderator", "subscriber"], "bits": 0, "color": "#1E90FF", "emotes": [["25", 6, 10], ["1902", 12, 16], ["25", 18, 22]], "first": false, "id": "abc-1", "k": "m", "login": "someone", "me": false, "name": "Some One", "reply": "", "text": "hello Kappa Keepo Kappa: done"}"##;
    let expected: Value = serde_json::from_str(python).unwrap();
    let ours = serde_json::to_value(message(MSG)).unwrap();
    assert_eq!(ours, expected);
}

#[test]
fn an_action_moves_the_emote_positions() {
    let ev =
        message("@emotes=25:8-12;display-name=A;id=x :a!a@a PRIVMSG #chan :\u{1}ACTION Kappa\u{1}");
    let Event::Message {
        me, text, emotes, ..
    } = ev
    else {
        panic!("not a message")
    };
    assert!(me);
    assert_eq!(text, "Kappa");
    assert_eq!(emotes, [("25".to_string(), 0, 4)]);
}

#[test]
fn a_colour_that_is_not_one_is_dropped() {
    let Event::Message { color, .. } =
        message("@color=red\\:url(x);display-name=A :a!a@a PRIVMSG #c :hi")
    else {
        panic!("not a message")
    };
    assert_eq!(color, "");
}

#[test]
fn notice_and_removals() {
    let ev = event(&parse_line(
        "@system-msg=A\\ssubscribed\\swith\\sPrime.;msg-id=sub :tmi.twitch.tv USERNOTICE #c :thanks",
    ));
    assert_eq!(
        ev,
        Some(Event::Notice {
            text: "A subscribed with Prime.".to_string(),
            said: "thanks".to_string()
        })
    );
    assert_eq!(
        serde_json::to_value(ev.unwrap()).unwrap(),
        json!({"k": "n", "text": "A subscribed with Prime.", "said": "thanks"})
    );
    assert_eq!(
        event(&parse_line(
            "@ban-duration=600 :tmi.twitch.tv CLEARCHAT #c :rude"
        )),
        Some(Event::Clear {
            login: "rude".to_string()
        })
    );
    assert_eq!(
        event(&parse_line(":tmi.twitch.tv CLEARCHAT #c")),
        Some(Event::Clear {
            login: String::new()
        })
    );
    assert_eq!(
        event(&parse_line(
            "@target-msg-id=abc-1 :tmi.twitch.tv CLEARMSG #c :hello"
        )),
        Some(Event::Delete {
            id: "abc-1".to_string()
        })
    );
    assert_eq!(
        event(&parse_line(":tmi.twitch.tv 001 justinfan1 :Welcome, GLHF!")),
        None
    );
}

#[test]
fn bad_emote_specs() {
    assert!(parse_emotes("").is_empty());
    assert_eq!(
        parse_emotes("25:9-2/../x:1/emotesv2_ab12:0-3"),
        [("emotesv2_ab12".to_string(), 0, 3)]
    );
}

#[test]
fn ping_roomstate_and_numbering() {
    assert_eq!(
        take(&parse_line("PING :tmi.twitch.tv")),
        Action::Reply("PONG :tmi.twitch.tv".to_string())
    );
    assert_eq!(
        take(&parse_line(
            "@room-id=71092938;slow=0 :tmi.twitch.tv ROOMSTATE #chan"
        )),
        Action::Joined {
            room_id: "71092938".to_string()
        }
    );
    let mut log = Log::default();
    for _ in 0..3 {
        let Action::Event(ev) = take(&parse_line(MSG)) else {
            panic!("not an event")
        };
        log.add(ev);
    }
    let numbers = |after: Option<u64>| log.since(after).iter().map(|e| e.n).collect::<Vec<_>>();
    assert_eq!(numbers(None), [1, 2, 3]);
    assert_eq!(log.seq(), 3);
    assert_eq!(numbers(Some(2)), [3]);
    assert!(numbers(Some(3)).is_empty());
    let first = serde_json::to_value(log.since(None)[0]).unwrap();
    assert_eq!(first["n"], 1);
    assert_eq!(first["k"], "m");
    assert_eq!(first["login"], "someone");
}

#[test]
fn a_number_from_before_a_restart_gets_the_recent_ones() {
    let mut log = Log::default();
    log.add(message(MSG));
    assert_eq!(log.since(Some(5000)).len(), 1);
}

#[test]
fn only_the_last_events_are_kept() {
    let mut log = Log::default();
    for _ in 0..KEEP + 30 {
        log.add(message(MSG));
    }
    assert_eq!(log.len(), KEEP);
    assert_eq!(log.since(None).len(), RECENT);
    assert_eq!(log.since(None)[0].n, (KEEP + 30 - RECENT + 1) as u64);
}

#[test]
fn reconnect_is_an_action_the_loop_handles() {
    assert_eq!(
        take(&parse_line(":tmi.twitch.tv RECONNECT")),
        Action::Reconnect
    );
    assert_eq!(
        take(&parse_line(":tmi.twitch.tv 001 justinfan1 :Welcome")),
        Action::Nothing
    );
}

#[test]
fn text_is_cut_to_six_hundred_characters() {
    let long = "é".repeat(700);
    let Event::Message { text, .. } = message(&format!(":a!a@a PRIVMSG #c :{long}")) else {
        panic!("not a message")
    };
    assert_eq!(text.chars().count(), 600);
}
