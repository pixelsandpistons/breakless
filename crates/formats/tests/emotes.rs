//! The three providers' answers and the merge, one test per case of the prototype's
//! `EmoteTests` (the cache and a provider away are the service's).

use breakless_formats::emotes::{Scope, Table, from_7tv, from_bttv, from_ffz, image_url, merge};
use serde_json::{Value, json};

fn bttv_global() -> Value {
    json!([{"id": "54fa8f1401e468494b85b537", "code": ":tf:", "imageType": "png", "modifier": false},
           {"id": "bad id!", "code": "Nope"}])
}
fn bttv_channel() -> Value {
    json!({"channelEmotes": [{"id": "aaa", "code": "catJAM"}], "sharedEmotes": [{"id": "bbb", "code": "monkaS"}]})
}
fn ffz_global() -> Value {
    json!({"default_sets": [3], "sets": {"3": {"emoticons": [{"id": 9, "name": "ZrehplaR"}]},
                                         "4": {"emoticons": [{"id": 10, "name": "NotDefault"}]}}})
}
fn ffz_room() -> Value {
    json!({"room": {}, "sets": {"166907": {"emoticons": [{"id": 77, "name": "catJAM"}, {"id": 78, "name": "OnlyFFZ"}]}}})
}
fn stv_global() -> Value {
    json!({"emotes": [{"id": "01A", "name": "reckH", "flags": 0}, {"id": "01B", "name": "RainTime", "flags": 1}]})
}
fn stv_user() -> Value {
    json!({"emote_set": {"emotes": [{"id": "01C", "name": "catJAM", "flags": 0}]}})
}

fn table(rows: &[(&str, char, &str, bool)]) -> Table {
    rows.iter()
        .map(|(w, p, id, z)| ((*w).to_string(), (*p, (*id).to_string(), *z)))
        .collect()
}

#[test]
fn readers() {
    assert_eq!(
        from_bttv(&bttv_global()),
        table(&[(":tf:", 'b', "54fa8f1401e468494b85b537", false)])
    );
    assert_eq!(
        from_bttv(&bttv_channel()),
        table(&[("catJAM", 'b', "aaa", false), ("monkaS", 'b', "bbb", false)])
    );
    assert_eq!(
        from_ffz(&ffz_global()),
        table(&[("ZrehplaR", 'f', "9", false)])
    );
    assert_eq!(
        from_ffz(&ffz_room()).keys().collect::<Vec<_>>(),
        ["OnlyFFZ", "catJAM"]
    );
    assert_eq!(
        from_7tv(&stv_global()),
        table(&[("reckH", '7', "01A", false), ("RainTime", '7', "01B", true)])
    );
}

fn all_six() -> Vec<(Scope, char, Table)> {
    vec![
        (Scope::Global, 'f', from_ffz(&ffz_global())),
        (Scope::Global, 'b', from_bttv(&bttv_global())),
        (Scope::Global, '7', from_7tv(&stv_global())),
        (Scope::Channel, 'f', from_ffz(&ffz_room())),
        (Scope::Channel, 'b', from_bttv(&bttv_channel())),
        (Scope::Channel, '7', from_7tv(&stv_user())),
    ]
}

#[test]
fn merge_channel_wins_and_7tv_wins_a_shared_word() {
    let (e, counts) = merge(&all_six());
    assert_eq!(e["catJAM"], ('7', "01C".to_string(), false));
    assert_eq!(
        (e["OnlyFFZ"].0, e["monkaS"].0, e[":tf:"].0, e["ZrehplaR"].0),
        ('f', 'b', 'b', 'f')
    );
    assert_eq!((counts[&'b'].global, counts[&'b'].channel), (1, 2));
}

#[test]
fn the_merge_serialises_as_the_prototype_does() {
    // Produced once by `reference/python/chat.py`:
    //   chat.emotes("71092938", fetch=test_chat.EmoteTests().fetch), json.dumps(..., sort_keys=True)
    let python = r#"{"counts": {"7": {"channel": 1, "global": 2}, "b": {"channel": 2, "global": 1}, "f": {"channel": 2, "global": 1}}, "emotes": {":tf:": ["b", "54fa8f1401e468494b85b537", false], "OnlyFFZ": ["f", "78", false], "RainTime": ["7", "01B", true], "ZrehplaR": ["f", "9", false], "catJAM": ["7", "01C", false], "monkaS": ["b", "bbb", false], "reckH": ["7", "01A", false]}, "errors": {}, "room_id": "71092938"}"#;
    let expected: Value = serde_json::from_str(python).unwrap();
    let (e, counts) = merge(&all_six());
    let ours = json!({"room_id": "71092938", "emotes": e, "counts": counts, "errors": {}});
    assert_eq!(ours, expected);
}

#[test]
fn without_a_room_number_only_the_globals() {
    let (e, _) = merge(&all_six()[..3]);
    assert!(!e.contains_key("catJAM"));
    assert!(e.contains_key("reckH"));
}

#[test]
fn image_addresses() {
    assert_eq!(
        image_url('b', "abc").as_deref(),
        Some("https://cdn.betterttv.net/emote/abc/2x.webp")
    );
    assert_eq!(
        image_url('t', "emotesv2_ab12").as_deref(),
        Some("https://static-cdn.jtvnw.net/emoticons/v2/emotesv2_ab12/default/dark/2.0")
    );
    for (p, id) in [
        ('x', "abc"),
        ('b', "../x"),
        ('b', ""),
        ('7', "a/b"),
        ('f', "a?b"),
    ] {
        assert_eq!(image_url(p, id), None, "{p} {id}");
    }
}
