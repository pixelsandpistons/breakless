//! Innertube client definitions read on 2026-10-31 from yt-dlp commit
//! `b77c9304e1c4220cca450c583491904daf8455af`, `yt_dlp/extractor/youtube/_base.py`.
//! Refresh the four `INNERTUBE_CLIENTS` entries together with their fixed request tests.

use clap::ValueEnum;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, ORIGIN, USER_AGENT};
use serde_json::{Value, json};

/// The four client identities measured independently by the probe.
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(crate) enum Client {
    /// The web player using the Safari user agent.
    #[default]
    #[value(name = "web_safari")]
    WebSafari,
    /// The television HTML5 player.
    Tv,
    /// The iOS application player.
    Ios,
    /// The mobile web player.
    Mweb,
}

impl Client {
    /// The command-line client name used in reports and capture file names.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::WebSafari => "web_safari",
            Self::Tv => "tv",
            Self::Ios => "ios",
            Self::Mweb => "mweb",
        }
    }

    fn definition(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::WebSafari => (
                "WEB",
                "2.20260708.00.00",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)",
                "1",
            ),
            Self::Tv => (
                "TVHTML5",
                "7.20260707.07.00",
                "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)",
                "7",
            ),
            Self::Ios => (
                "IOS",
                "21.26.4",
                "com.google.ios.youtube/21.26.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
                "5",
            ),
            Self::Mweb => (
                "MWEB",
                "2.20260708.05.00",
                "Mozilla/5.0 (iPad; CPU OS 16_7_10 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1,gzip(gfe)",
                "2",
            ),
        }
    }

    /// The client context, including yt-dlp's English language and UTC extraction settings.
    pub(crate) fn context(self) -> Value {
        let (name, version, user_agent, _) = self.definition();
        let mut client = json!({
            "clientName": name,
            "clientVersion": version,
            "userAgent": user_agent,
            "hl": "en",
            "timeZone": "UTC",
            "utcOffsetMinutes": 0,
        });
        if matches!(self, Self::Ios) {
            client["deviceMake"] = json!("Apple");
            client["deviceModel"] = json!("iPhone16,2");
            client["osName"] = json!("iPhone");
            client["osVersion"] = json!("18.3.2.22D82");
        }
        json!({"client": client})
    }

    /// The static player request body, without a token, account state or JavaScript-derived data.
    pub(crate) fn player_body(self, video_id: &str) -> Value {
        json!({
            "context": self.context(),
            "videoId": video_id,
            "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}},
            "contentCheckOk": true,
            "racyCheckOk": true,
        })
    }

    /// The identity headers yt-dlp sends for an unauthenticated Innertube request.
    pub(crate) fn headers(self) -> HeaderMap {
        let (_, version, user_agent, number) = self.definition();
        HeaderMap::from_iter([
            (
                HeaderName::from_static("x-youtube-client-name"),
                HeaderValue::from_static(number),
            ),
            (
                HeaderName::from_static("x-youtube-client-version"),
                HeaderValue::from_static(version),
            ),
            (USER_AGENT, HeaderValue::from_static(user_agent)),
            (ORIGIN, HeaderValue::from_static("https://www.youtube.com")),
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safari_player_body_matches_the_fixed_upstream_client() {
        assert_eq!(
            Client::WebSafari.player_body("test_video1"),
            json!({
                "context": {"client": {
                    "clientName": "WEB",
                    "clientVersion": "2.20260708.00.00",
                    "userAgent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)",
                    "hl": "en", "timeZone": "UTC", "utcOffsetMinutes": 0
                }},
                "videoId": "test_video1",
                "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}},
                "contentCheckOk": true, "racyCheckOk": true
            })
        );
    }

    #[test]
    fn television_player_body_matches_the_fixed_upstream_client() {
        assert_eq!(
            Client::Tv.player_body("test_video1"),
            json!({
                "context": {"client": {
                    "clientName": "TVHTML5",
                    "clientVersion": "7.20260707.07.00",
                    "userAgent": "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)",
                    "hl": "en", "timeZone": "UTC", "utcOffsetMinutes": 0
                }},
                "videoId": "test_video1",
                "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}},
                "contentCheckOk": true, "racyCheckOk": true
            })
        );
    }

    #[test]
    fn ios_player_body_matches_the_fixed_upstream_client_and_device() {
        assert_eq!(
            Client::Ios.player_body("test_video1"),
            json!({
                "context": {"client": {
                    "clientName": "IOS",
                    "clientVersion": "21.26.4",
                    "userAgent": "com.google.ios.youtube/21.26.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
                    "deviceMake": "Apple", "deviceModel": "iPhone16,2", "osName": "iPhone", "osVersion": "18.3.2.22D82",
                    "hl": "en", "timeZone": "UTC", "utcOffsetMinutes": 0
                }},
                "videoId": "test_video1",
                "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}},
                "contentCheckOk": true, "racyCheckOk": true
            })
        );
    }

    #[test]
    fn mobile_web_player_body_matches_the_fixed_upstream_client() {
        assert_eq!(
            Client::Mweb.player_body("test_video1"),
            json!({
                "context": {"client": {
                    "clientName": "MWEB",
                    "clientVersion": "2.20260708.05.00",
                    "userAgent": "Mozilla/5.0 (iPad; CPU OS 16_7_10 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1,gzip(gfe)",
                    "hl": "en", "timeZone": "UTC", "utcOffsetMinutes": 0
                }},
                "videoId": "test_video1",
                "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}},
                "contentCheckOk": true, "racyCheckOk": true
            })
        );
    }

    #[test]
    fn identity_headers_match_fixed_upstream_numbers_versions_and_user_agents() {
        for (client, name, number, version, agent) in [
            (
                Client::WebSafari,
                "web_safari",
                "1",
                "2.20260708.00.00",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)",
            ),
            (
                Client::Tv,
                "tv",
                "7",
                "7.20260707.07.00",
                "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)",
            ),
            (
                Client::Ios,
                "ios",
                "5",
                "21.26.4",
                "com.google.ios.youtube/21.26.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
            ),
            (
                Client::Mweb,
                "mweb",
                "2",
                "2.20260708.05.00",
                "Mozilla/5.0 (iPad; CPU OS 16_7_10 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1,gzip(gfe)",
            ),
        ] {
            let headers = client.headers();
            assert_eq!(client.name(), name);
            assert_eq!(headers["x-youtube-client-name"], number);
            assert_eq!(headers["x-youtube-client-version"], version);
            assert_eq!(headers["user-agent"], agent);
            assert_eq!(headers["origin"], "https://www.youtube.com");
            assert_eq!(headers["content-type"], "application/json");
            assert_eq!(headers.len(), 5);
        }
    }
}
