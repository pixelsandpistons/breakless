use std::fmt::Write as _;

use crate::{FakeTwitch, Script, WINDOW};

impl FakeTwitch {
    pub(crate) fn master(&self, login: &str) -> Option<String> {
        let inner = self.inner();
        let channel = inner.channels.get(login).filter(|c| !c.offline)?;
        Some(format!(
            "#EXTM3U\n#EXT-X-TWITCH-INFO:NODE=\"fake\",USER-COUNTRY=\"{cc}\",CLUSTER=\"fake_cluster\"\n\
             #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"chunked\",NAME=\"1080p60 (source)\",AUTOSELECT=YES,DEFAULT=YES\n\
             #EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080,CODECS=\"avc1.64002A,mp4a.40.2\",VIDEO=\"chunked\",FRAME-RATE=60.000\n\
             http://{cdn}/media/{login}/chunked.m3u8\n\
             #EXT-X-MEDIA:TYPE=VIDEO,GROUP-ID=\"720p60\",NAME=\"720p60\",AUTOSELECT=YES,DEFAULT=YES\n\
             #EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720,CODECS=\"avc1.4D401F,mp4a.40.2\",VIDEO=\"720p60\",FRAME-RATE=60.000\n\
             http://{cdn}/media/{login}/720p60.m3u8\n",
            cc = channel.country,
            cdn = self.cdn
        ))
    }

    pub(crate) fn media(&self, login: &str) -> Option<String> {
        let mut inner = self.inner();
        let channel = inner.channels.get_mut(login).filter(|c| !c.offline)?;
        channel.sync();
        Some(render(channel, login, self.cdn))
    }
}

fn render(channel: &Script, login: &str, cdn: std::net::SocketAddr) -> String {
    let mut start = channel.pos.saturating_sub(WINDOW);
    if let Some(advert) = channel.breaks.last() {
        // Keep the first advert playlist and the resumed stream separate, even for short breaks.
        start = start.max(if channel.pos <= advert.end {
            advert.start
        } else {
            advert.end
        });
    }
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:{start}\n"
    );
    let mut i = start;
    while i < channel.pos {
        if channel.is_ad(i) {
            let first = i;
            while i < channel.pos && channel.is_ad(i) {
                i += 1;
            }
            let roll = channel
                .breaks
                .iter()
                .find(|b| b.start <= first && first < b.end)
                .map_or_else(
                    || if first == 0 { "PREROLL" } else { "MIDROLL" },
                    |b| b.roll.as_str(),
                );
            writeln!(out,
                "#EXT-X-DATERANGE:ID=\"stitched-ad-{first}\",CLASS=\"twitch-stitched-ad\",START-DATE=\"{}\",DURATION={}.000,X-TV-TWITCH-AD-ROLL-TYPE=\"{roll}\",X-TV-TWITCH-AD-POD-FILLED-DURATION=\"{}\"",
                when(first), 2 * (i-first), 2 * (i-first)).expect("writing a playlist string");
        } else {
            i += 1;
        }
    }
    let mut previous = None;
    for i in start..channel.pos {
        let ad = channel.is_ad(i);
        if previous.is_some_and(|p| p != ad) || channel.breaks.iter().any(|b| b.end == i) {
            out.push_str("#EXT-X-DISCONTINUITY\n");
        }
        previous = Some(ad);
        writeln!(
            out,
            "#EXT-X-PROGRAM-DATE-TIME:{}\n#EXTINF:2.000,{}",
            when(i),
            if ad { "Amazon|1" } else { "live" }
        )
        .expect("writing a playlist string");
        let kind = if ad { "ad" } else { "live" };
        if channel.legacy {
            writeln!(out, "http://cdn.example/{login}/{kind}-{i}.ts")
                .expect("writing a playlist string");
        } else {
            writeln!(out, "http://{cdn}/segments/{login}/{kind}-{i}.ts")
                .expect("writing a playlist string");
        }
    }
    out
}

fn when(i: usize) -> String {
    let seconds = i.saturating_mul(2);
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX - 719_468);
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}
