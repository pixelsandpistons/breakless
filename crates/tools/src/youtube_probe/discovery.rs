//! Live candidates from Innertube's search renderers, without fetching video pages.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::client::Client;

/// A search for `live`, restricted to video results as in yt-dlp's search extractor.
/// Continuation data augments the same search body.
pub(super) fn search_body(continuation: Option<&str>) -> Value {
    let mut body = json!({
        "context": Client::WebSafari.context(),
        "query": "live",
        "params": "EgIQAfABAQ==",
    });
    if let Some(token) = continuation {
        body["continuation"] = json!(token);
    }
    body
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn contains_field(value: &Value, key: &str, expected: &str) -> bool {
    match value {
        Value::Object(object) => {
            object.get(key).and_then(Value::as_str) == Some(expected)
                || object.values().any(|v| contains_field(v, key, expected))
        }
        Value::Array(array) => array.iter().any(|v| contains_field(v, key, expected)),
        _ => false,
    }
}

fn text(value: &Value) -> String {
    if let Some(simple) = value.get("simpleText").and_then(Value::as_str) {
        return simple.to_string();
    }
    value["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|run| run["text"].as_str())
        .collect()
}

fn live_renderer(renderer: &Value) -> bool {
    if renderer.get("upcomingEventData").is_some()
        || text(&renderer["publishedTimeText"])
            .to_lowercase()
            .contains("streamed")
        || text(&renderer["videoInfo"])
            .to_lowercase()
            .contains("streamed")
    {
        return false;
    }
    contains_field(&renderer["thumbnailOverlays"], "style", "LIVE")
        || contains_field(&renderer["badges"], "style", "BADGE_STYLE_TYPE_LIVE_NOW")
}

fn live_lockup(renderer: &Value) -> bool {
    renderer["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO"
        && !contains_field(renderer, "text", "upcoming")
        && contains_field(renderer, "badgeStyle", "THUMBNAIL_OVERLAY_BADGE_STYLE_LIVE")
}

fn collect_ids(value: &Value, seen: &mut BTreeSet<String>, ids: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            let candidates = [
                object
                    .get("videoRenderer")
                    .filter(|r| live_renderer(r))
                    .map(|r| &r["videoId"]),
                object
                    .get("lockupViewModel")
                    .filter(|r| live_lockup(r))
                    .map(|r| &r["contentId"]),
            ];
            for id in candidates.into_iter().flatten().filter_map(Value::as_str) {
                if valid_id(id) && seen.insert(id.to_string()) {
                    ids.push(id.to_string());
                }
            }
            for child in object.values() {
                collect_ids(child, seen, ids);
            }
        }
        Value::Array(array) => {
            for child in array {
                collect_ids(child, seen, ids);
            }
        }
        _ => {}
    }
}

/// Unique live-marked video IDs, preserving the search result array's order.
pub(super) fn live_ids(response: &Value) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut ids = Vec::new();
    collect_ids(response, &mut seen, &mut ids);
    ids
}

/// The next search page's continuation token, when one was returned.
pub(super) fn continuation(response: &Value) -> Option<String> {
    match response {
        Value::Object(object) => {
            let token = object
                .get("continuationItemRenderer")
                .and_then(|v| v.pointer("/continuationEndpoint/continuationCommand/token"))
                .or_else(|| {
                    object
                        .get("nextContinuationData")
                        .and_then(|v| v.get("continuation"))
                })
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty());
            token
                .map(str::to_string)
                .or_else(|| object.values().find_map(continuation))
        }
        Value::Array(array) => array.iter().find_map(continuation),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_body_uses_the_live_query_videos_filter_and_optional_continuation() {
        let expected = json!({
            "context": {"client": {
                "clientName": "WEB",
                "clientVersion": "2.20260708.00.00",
                "userAgent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)",
                "hl": "en",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
            }},
            "query": "live",
            "params": "EgIQAfABAQ==",
        });
        assert_eq!(search_body(None), expected);
        let mut continued = expected;
        continued["continuation"] = json!("next-page");
        assert_eq!(search_body(Some("next-page")), continued);
    }

    fn overlay(id: &str) -> Value {
        json!({"videoRenderer": {
            "videoId": id,
            "thumbnailOverlays": [{"thumbnailOverlayTimeStatusRenderer": {"style": "LIVE"}}],
        }})
    }

    #[test]
    fn live_candidates_exclude_recordings_upcoming_invalid_and_duplicate_ids() {
        let first = overlay("abcdefghijk");
        let mut upcoming = overlay("upcoming123");
        upcoming["videoRenderer"]["upcomingEventData"] = json!({"startTime": "9999999999"});
        let mut recorded = overlay("recorded123");
        recorded["videoRenderer"]["publishedTimeText"] =
            json!({"simpleText": "Streamed 2 days ago"});
        let badge = json!({"videoRenderer": {
            "videoId": "0123456789_",
            "badges": [{"metadataBadgeRenderer": {"style": "BADGE_STYLE_TYPE_LIVE_NOW"}}],
        }});
        let response = json!({"contents": [
            first.clone(), upcoming, recorded,
            {"videoRenderer": {"videoId": "ordinary123"}},
            overlay("too-short"), overlay("invalid$id!"),
            badge, first,
        ]});
        assert_eq!(live_ids(&response), ["abcdefghijk", "0123456789_"]);
    }

    #[test]
    fn live_video_lockups_are_selected_but_playlists_and_upcoming_are_not() {
        let live = json!({"lockupViewModel": {
            "contentId": "ZYXWVUTSRQP",
            "contentType": "LOCKUP_CONTENT_TYPE_VIDEO",
            "contentImage": {"thumbnailViewModel": {"overlays": [{
                "thumbnailOverlayBadgeViewModel": {"thumbnailBadges": [{
                    "thumbnailBadgeViewModel": {"badgeStyle": "THUMBNAIL_OVERLAY_BADGE_STYLE_LIVE", "text": "LIVE"},
                }]},
            }]}},
        }});
        let mut playlist = live.clone();
        playlist["lockupViewModel"]["contentType"] = json!("LOCKUP_CONTENT_TYPE_PLAYLIST");
        let mut upcoming = live.clone();
        upcoming["lockupViewModel"]["badge"] = json!({"text": "upcoming"});
        assert_eq!(
            live_ids(&json!([playlist, live, upcoming])),
            ["ZYXWVUTSRQP"]
        );
    }

    #[test]
    fn continuation_tokens_are_found_in_both_search_page_shapes() {
        let response = json!({"contents": [{"continuationItemRenderer": {
            "continuationEndpoint": {"continuationCommand": {"token": "page-two"}},
        }}]});
        assert_eq!(continuation(&response).as_deref(), Some("page-two"));
        assert_eq!(
            continuation(&json!({"continuations": [{"nextContinuationData": {"continuation": "page-three"}}]})).as_deref(),
            Some("page-three")
        );
        assert_eq!(continuation(&json!({"token": "unrelated"})), None);
        assert_eq!(continuation(&Value::Null), None);
    }
}
