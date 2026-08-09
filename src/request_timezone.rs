//! Java-compatible request timezone and `<lor:date*>` rendering support.

use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Duration, Timelike, Utc};
use once_cell::sync::Lazy;
use regex::{Captures, Regex};

static TIME_TAG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?s)<time data-format="(?P<format>[^"]+)" datetime="(?P<datetime>[^"]+)"(?P<extra>[^>]*)>.*?</time>"#,
    )
    .expect("time tag regex")
});

pub fn stRequestTimezone(stJar: &CookieJar) -> chrono_tz::Tz {
    stJar
        .get("tz")
        .map(|stCookie| stCookie.value())
        .filter(|sTimezone| !sTimezone.is_empty())
        .filter(|sTimezone| !matches!(*sTimezone, "Factory" | "Etc/Unknown"))
        .and_then(|sTimezone| sTimezone.parse().ok())
        .or_else(|| {
            std::env::var("TZ")
                .ok()
                .filter(|sTimezone| !sTimezone.is_empty())
                .filter(|sTimezone| !matches!(sTimezone.as_str(), "Factory" | "Etc/Unknown"))
                .and_then(|sTimezone| sTimezone.parse().ok())
        })
        // The Java fallback is ZoneId.systemDefault().  The runtime image is
        // UTC unless an operator explicitly provides TZ.
        .unwrap_or(chrono_tz::Etc::UTC)
}

/// Build the neutral server-side representation consumed by
/// [`sRewriteHtmlTimes`]. This is useful for route handlers which assemble
/// small HTML fragments without an Askama template.
pub fn sTimeTag(sFormat: &str, dtValue: DateTime<Utc>) -> String {
    debug_assert!(matches!(
        sFormat,
        "default" | "date" | "interval" | "compact-interval"
    ));
    format!(
        r#"<time data-format="{sFormat}" datetime="{}">{dtValue}</time>"#,
        dtValue.to_rfc3339()
    )
}

fn sFormatInterval(
    dtValue: DateTime<Utc>,
    stTimezone: chrono_tz::Tz,
    dtNow: DateTime<Utc>,
    bCompact: bool,
) -> String {
    let iMillis = (dtNow - dtValue).num_milliseconds();
    let dtLocal = dtValue.with_timezone(&stTimezone);
    let dtLocalNow = dtNow.with_timezone(&stTimezone);
    let dtToday = dtLocalNow
        .with_hour(0)
        .and_then(|dt| dt.with_minute(0))
        .and_then(|dt| dt.with_second(0))
        .and_then(|dt| dt.with_nanosecond(0))
        .expect("valid local midnight");
    let dtYesterday = dtToday - Duration::days(1);

    if bCompact {
        if iMillis < 60 * 60 * 1000 {
            return format!("{}&nbsp;мин", std::cmp::max(1, iMillis / (60 * 1000)));
        }
        if iMillis < 4 * 60 * 60 * 1000 || dtLocal > dtToday {
            return dtLocal.format("%H:%M").to_string();
        }
        if dtLocal > dtYesterday {
            return "вчера".to_owned();
        }
        return dtLocal.format("%d.%m.%y").to_string();
    }

    if iMillis < 2 * 60 * 1000 {
        return "минуту назад".to_owned();
    }
    if iMillis < 60 * 60 * 1000 {
        let iMinutes = iMillis / (60 * 1000);
        let sEnding = if iMinutes % 10 < 5 && iMinutes % 10 > 1 && !(10..=20).contains(&iMinutes) {
            "минуты"
        } else if iMinutes % 10 == 1 && iMinutes > 20 {
            "минута"
        } else {
            "минут"
        };
        return format!("{iMinutes}&nbsp;{sEnding} назад");
    }
    if dtLocal > dtToday {
        return format!("сегодня {}", dtLocal.format("%H:%M"));
    }
    if dtLocal > dtYesterday {
        return format!("вчера {}", dtLocal.format("%H:%M"));
    }
    dtLocal.format("%d.%m.%y %H:%M").to_string()
}

fn sFormatTime(
    sFormat: &str,
    dtValue: DateTime<Utc>,
    stTimezone: chrono_tz::Tz,
    dtNow: DateTime<Utc>,
) -> Option<String> {
    match sFormat {
        "default" => {
            let dtLocal = dtValue.with_timezone(&stTimezone);
            let sShortZone = dtLocal.format("%Z").to_string();
            let sZone = if matches!(sShortZone.as_bytes().first(), Some(b'+') | Some(b'-')) {
                format!("GMT{}", dtLocal.format("%:z"))
            } else {
                sShortZone
            };
            Some(format!("{} {sZone}", dtLocal.format("%d.%m.%y %H:%M:%S")))
        }
        "date" => Some(
            dtValue
                .with_timezone(&stTimezone)
                .format("%d.%m.%y")
                .to_string(),
        ),
        "interval" => Some(sFormatInterval(dtValue, stTimezone, dtNow, false)),
        "compact-interval" => Some(sFormatInterval(dtValue, stTimezone, dtNow, true)),
        _ => None,
    }
}

pub fn sRewriteHtmlTimes(sHtml: &str, stTimezone: chrono_tz::Tz, dtNow: DateTime<Utc>) -> String {
    TIME_TAG_RE
        .replace_all(sHtml, |stCaptures: &Captures<'_>| {
            let sOriginal = stCaptures.get(0).map_or("", |stMatch| stMatch.as_str());
            let sFormat = stCaptures
                .name("format")
                .map_or("", |stMatch| stMatch.as_str());
            let sDateTime = stCaptures
                .name("datetime")
                .map_or("", |stMatch| stMatch.as_str());
            let Ok(dtValue) = DateTime::parse_from_rfc3339(sDateTime) else {
                return sOriginal.to_owned();
            };
            let dtValue = dtValue.with_timezone(&Utc);
            let Some(sValue) = sFormatTime(sFormat, dtValue, stTimezone, dtNow) else {
                return sOriginal.to_owned();
            };
            let sExtra = stCaptures
                .name("extra")
                .map_or("", |stMatch| stMatch.as_str());
            let sIso = dtValue
                .with_timezone(&chrono_tz::Europe::Moscow)
                .to_rfc3339();
            format!(r#"<time data-format="{sFormat}" datetime="{sIso}"{sExtra}>{sValue}</time>"#)
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_extra::extract::cookie::Cookie;
    use chrono::TimeZone;

    #[test]
    fn cookie_rejects_java_bad_timezones() {
        let stJar = CookieJar::new().add(Cookie::new("tz", "Etc/Unknown"));
        assert_ne!(stRequestTimezone(&stJar).name(), "Etc/Unknown");
        let stJar = CookieJar::new().add(Cookie::new("tz", "Asia/Yekaterinburg"));
        assert_eq!(stRequestTimezone(&stJar), chrono_tz::Asia::Yekaterinburg);
    }

    #[test]
    fn rewrites_java_date_and_interval_contracts() {
        let dtNow = Utc.with_ymd_and_hms(2026, 8, 9, 0, 0, 0).unwrap();
        let sHtml = r#"<time data-format="default" datetime="2026-08-08T20:30:00+00:00">raw</time> <time data-format="compact-interval" datetime="2026-08-08T23:55:00Z">raw</time>"#;
        let sActual = sRewriteHtmlTimes(sHtml, chrono_tz::Europe::Moscow, dtNow);
        assert!(sActual.contains("08.08.26 23:30:00 MSK"));
        assert!(sActual.contains("5&nbsp;мин"));
        assert!(sActual.contains("datetime=\"2026-08-08T23:30:00+03:00\""));
    }

    #[test]
    fn unnamed_zone_uses_java_gmt_offset_label() {
        let dtNow = Utc.with_ymd_and_hms(2007, 5, 17, 12, 0, 0).unwrap();
        let sHtml = r#"<time data-format="default" datetime="2007-05-17T10:39:20Z">raw</time>"#;
        let sActual = sRewriteHtmlTimes(sHtml, chrono_tz::Asia::Yekaterinburg, dtNow);
        assert!(sActual.contains("17.05.07 16:39:20 GMT+06:00"));
    }

    #[test]
    fn helper_emits_a_rewritable_time_tag() {
        let dtValue = Utc.with_ymd_and_hms(2026, 8, 9, 0, 0, 0).unwrap();
        let sTag = sTimeTag("interval", dtValue);
        assert!(
            sTag.starts_with(
                r#"<time data-format="interval" datetime="2026-08-09T00:00:00+00:00">"#
            )
        );
        assert!(sTag.ends_with(" UTC</time>"));
    }
}
