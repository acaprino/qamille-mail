//! Shared regional settings. Time zone rules are bundled for offline use, while
//! iana-time-zone maps the operating system's zone (including Windows) to IANA.
use crate::domain::model::Settings;
use chrono::TimeZone;
#[cfg(test)]
use chrono::NaiveDate;
pub use crate::domain::regional::day_start;
use chrono_tz::Tz;

pub fn resolved_timezone(settings: &Settings) -> String {
    let name = if settings.time_zone.is_empty() {
        iana_time_zone::get_timezone().unwrap_or_default()
    } else {
        settings.time_zone.clone()
    };
    name.parse::<Tz>().map(|zone| zone.name().to_owned()).unwrap_or_else(|_| "UTC".into())
}

pub fn timezone_warning(settings: &Settings) -> Option<String> {
    let candidate = if settings.time_zone.is_empty() {
        iana_time_zone::get_timezone().unwrap_or_default()
    } else { settings.time_zone.clone() };
    candidate.parse::<Tz>().err().map(|_| "Fuso non riconosciuto: uso UTC / Unknown time zone: using UTC".into())
}

pub fn resolved_locale(settings: &Settings) -> String {
    if settings.locale.is_empty() {
        sys_locale::get_locale().filter(|locale| valid_locale(locale)).unwrap_or_else(|| "en-US".into())
    } else if valid_locale(&settings.locale) { settings.locale.clone() }
    else { "en-US".into() }
}

pub fn valid_locale(locale: &str) -> bool {
    let mut parts = locale.split('-');
    parts.next().is_some_and(|language| (2..=3).contains(&language.len()) && language.bytes().all(|c| c.is_ascii_alphabetic()))
        && parts.all(|part| (2..=8).contains(&part.len()) && part.bytes().all(|c| c.is_ascii_alphanumeric()))
}

pub fn format_timestamp(settings: &Settings, timestamp: i64) -> String {
    let zone = resolved_timezone(settings).parse::<Tz>().unwrap_or(chrono_tz::UTC);
    zone.timestamp_opt(timestamp, 0).single().map(|time| {
        let locale = resolved_locale(settings);
        let format = if locale.starts_with("en-US") { "%m/%d/%Y %I:%M %p %Z" } else { "%d/%m/%Y %H:%M %Z" };
        time.format(format).to_string()
    }).unwrap_or_else(|| "Date unavailable".into())
}

pub fn zone(settings: &Settings) -> Tz {
    resolved_timezone(settings).parse().unwrap_or(chrono_tz::UTC)
}

pub fn parse_when(settings: &Settings, value: &str) -> Result<i64, String> {
    crate::domain::regional::parse_when(value, crate::model::now_ts(),
        &resolved_timezone(settings), &resolved_locale(settings))
}

pub fn apply_email_action(store: &mut crate::model::Store, id: u64, action: &str, value: Option<&str>) -> Result<String, String> {
    crate::actions::apply_at(store, id, action, value, crate::model::now_ts(),
        &resolved_timezone(&store.settings), &resolved_locale(&store.settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::LocalResult;
    fn date(value: &str) -> NaiveDate { NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap() }
    #[test]
    fn rome_fall_day_has_twenty_five_hours() {
        let start = day_start(date("2026-10-25"), "Europe/Rome").unwrap();
        let end = day_start(date("2026-10-26"), "Europe/Rome").unwrap();
        assert_eq!(end - start, 25 * 3600);
        assert_eq!(chrono::DateTime::from_timestamp(start, 0).unwrap().to_rfc3339(), "2026-10-24T22:00:00+00:00");
    }
    #[test]
    fn missing_midnight_starts_at_first_valid_instant() {
        let start = day_start(date("2026-03-08"), "America/Havana").unwrap();
        assert_eq!(chrono::DateTime::from_timestamp(start, 0).unwrap().to_rfc3339(), "2026-03-08T05:00:00+00:00");
    }
    #[test]
    fn repeated_midnight_uses_first_occurrence() {
        let day = date("2026-11-01");
        let zone = chrono_tz::America::Havana;
        let midnight = day.and_hms_opt(0, 0, 0).unwrap();
        let LocalResult::Ambiguous(first, second) = zone.from_local_datetime(&midnight) else { panic!("expected repeated midnight") };
        assert_eq!(day_start(day, zone.name()).unwrap(), first.timestamp().min(second.timestamp()));
    }
    #[test]
    fn unknown_setting_is_visible_utc_fallback() {
        let settings = Settings { time_zone: "Somewhere/Unknown".into(), ..Settings::default() };
        assert_eq!(resolved_timezone(&settings), "UTC");
        assert!(timezone_warning(&settings).is_some());
        assert!(day_start(date("2026-10-15"), "Somewhere/Unknown").is_err());
    }
    #[test]
    fn completely_skipped_date_has_empty_interval() {
        assert_eq!(day_start(date("2011-12-30"), "Pacific/Apia").unwrap(), day_start(date("2011-12-31"), "Pacific/Apia").unwrap());
    }
}
