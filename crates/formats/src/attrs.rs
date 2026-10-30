//! HLS attribute lists (`KEY=value,KEY="quoted value"`) and the dates Twitch writes in them.

/// The value of `key` in an HLS attribute list, or `None` when the key is absent.
///
/// A key counts only at the start of the list or after `,` or `:`, so `DURATION` does not match
/// inside `X-TV-TWITCH-AD-POD-FILLED-DURATION`. A quoted value is returned without its quotes; an
/// unquoted one runs to the next comma or the end of the line.
pub fn attr<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(pos) = line[from..].find(key) {
        let start = from + pos;
        let bounded = start == 0 || matches!(line.as_bytes()[start - 1], b',' | b':');
        let rest = &line[start + key.len()..];
        if bounded && rest.starts_with('=') {
            let value = &rest[1..];
            return Some(if let Some(quoted) = value.strip_prefix('"') {
                quoted.split('"').next().unwrap_or("")
            } else {
                value.split(',').next().unwrap_or("")
            });
        }
        from = start + key.len().max(1);
    }
    None
}

/// `2026-10-04T12:00:02.000Z` (or with a `+HH:MM` offset) as seconds since the Unix epoch.
///
/// Twitch writes every `START-DATE` and `#EXT-X-PROGRAM-DATE-TIME` this way. Anything else is
/// `None`, and a segment without a time simply cannot fall inside an advert window.
pub fn when(text: &str) -> Option<f64> {
    let text = text.trim();
    let (date, time) = text.split_once(['T', ' '])?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (time, offset) = split_offset(time)?;
    let mut parts = time.split(':');
    let hour: f64 = parts.next()?.parse().ok()?;
    let minute: f64 = parts.next()?.parse().ok()?;
    let second: f64 = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() || hour >= 24.0 || minute >= 60.0 || second >= 61.0 {
        return None;
    }
    let days = days_from_civil(year, month, day) as f64;
    Some(days * 86_400.0 + hour * 3_600.0 + minute * 60.0 + second - offset)
}

/// The time part without its zone, and the zone's offset in seconds east of UTC.
fn split_offset(time: &str) -> Option<(&str, f64)> {
    if let Some(bare) = time.strip_suffix('Z') {
        return Some((bare, 0.0));
    }
    if let Some(pos) = time.rfind(['+', '-']) {
        let (bare, zone) = time.split_at(pos);
        let sign = if zone.starts_with('-') { -1.0 } else { 1.0 };
        let (h, m) = zone[1..].split_once(':').unwrap_or((&zone[1..], "0"));
        let h: f64 = h.parse().ok()?;
        let m: f64 = m.parse().ok()?;
        return Some((bare, sign * (h * 3_600.0 + m * 60.0)));
    }
    Some((time, 0.0))
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_and_unquoted_values() {
        let line = r#"#EXT-X-DATERANGE:ID="stitched-ad-1",CLASS="twitch-stitched-ad",DURATION=29.297,X-TV-TWITCH-AD-POD-FILLED-DURATION="60""#;
        assert_eq!(attr(line, "ID"), Some("stitched-ad-1"));
        assert_eq!(attr(line, "CLASS"), Some("twitch-stitched-ad"));
        assert_eq!(attr(line, "DURATION"), Some("29.297"));
        assert_eq!(attr(line, "X-TV-TWITCH-AD-POD-FILLED-DURATION"), Some("60"));
        assert_eq!(attr(line, "MISSING"), None);
    }

    #[test]
    fn a_key_inside_a_longer_key_does_not_count() {
        let line = r#"#EXT-X-DATERANGE:X-TV-TWITCH-AD-POD-FILLED-DURATION="60",ID="x""#;
        assert_eq!(attr(line, "DURATION"), None);
        assert_eq!(
            attr(
                "#EXT-X-STREAM-INF:AVERAGE-BANDWIDTH=100,BANDWIDTH=200",
                "BANDWIDTH"
            ),
            Some("200")
        );
    }

    #[test]
    fn dates_twitch_writes() {
        let epoch = when("1970-01-01T00:00:00Z").unwrap();
        assert!(epoch.abs() < f64::EPSILON);
        let t = when("2026-10-04T12:00:02.000Z").unwrap();
        assert!((t - 1_791_115_202.0).abs() < 1e-6, "{t}");
        let plus = when("2026-10-04T13:00:02.000+01:00").unwrap();
        assert!((plus - t).abs() < 1e-6);
        assert_eq!(when("not a date"), None);
        assert_eq!(when("2026-13-04T12:00:02Z"), None);
    }
}
