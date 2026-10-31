use crate::FakeTwitch;
use serde_json::{Value, json};

impl FakeTwitch {
    pub(crate) fn emotes(&self, rest: &str) -> (u16, Value) {
        let mut inner = self.inner();
        inner.emote_requests.push(rest.to_string());
        if let Some((_, status)) = inner.emote_status.iter().find(|(p, _)| rest.contains(p)) {
            return (*status, Value::String("no".to_string()));
        }
        let table: &[(&str, Value)] = &[
            (
                "betterttv.net/3/cached/emotes/global",
                json!([{"id": "54fa8f1401e468494b85b537", "code": ":tf:", "imageType": "png", "modifier": false}, {"id": "bad id!", "code": "Nope"}]),
            ),
            (
                "betterttv.net/3/cached/users",
                json!({"channelEmotes": [{"id": "aaa", "code": "catJAM"}], "sharedEmotes": [{"id": "bbb", "code": "monkaS"}]}),
            ),
            (
                "frankerfacez.com/v1/set/global",
                json!({"default_sets": [3], "sets": {"3": {"emoticons": [{"id": 9, "name": "ZrehplaR"}]}, "4": {"emoticons": [{"id": 10, "name": "NotDefault"}]}}}),
            ),
            (
                "frankerfacez.com/v1/room",
                json!({"room": {}, "sets": {"166907": {"emoticons": [{"id": 77, "name": "catJAM"}, {"id": 78, "name": "OnlyFFZ"}]}}}),
            ),
            (
                "7tv.io/v3/emote-sets/global",
                json!({"emotes": [{"id": "01A", "name": "reckH", "flags": 0}, {"id": "01B", "name": "RainTime", "flags": 1}]}),
            ),
            (
                "7tv.io/v3/users",
                json!({"emote_set": {"emotes": [{"id": "01C", "name": "catJAM", "flags": 0}]}}),
            ),
        ];
        match table.iter().find(|(piece, _)| rest.contains(piece)) {
            Some((_, value)) => (200, value.clone()),
            None => (404, Value::String("no such list".to_string())),
        }
    }
}
