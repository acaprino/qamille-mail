//! Pure calendar and formatting rules. Callers supply resolved regional settings.
use crate::model::Settings;
use chrono::{Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone};
use chrono_tz::Tz;

pub fn valid_locale(locale: &str) -> bool {
    let mut parts = locale.split('-');
    parts.next().is_some_and(|language| {
        (2..=3).contains(&language.len()) && language.bytes().all(|c| c.is_ascii_alphabetic())
    }) && parts.all(|part| {
        (2..=8).contains(&part.len()) && part.bytes().all(|c| c.is_ascii_alphanumeric())
    })
}

/// Earliest valid instant on a local date; a skipped date shares the next boundary.
pub fn day_start(date: NaiveDate, zone: &str) -> Result<i64, String> {
    let zone: Tz = zone
        .parse()
        .map_err(|_| format!("Unknown time zone: {zone}"))?;
    let midnight = date.and_hms_opt(0, 0, 0).ok_or("Date out of range")?;
    for seconds in 0..=172_800 {
        let local = midnight
            .checked_add_signed(Duration::seconds(seconds))
            .ok_or("Date out of range")?;
        match zone.from_local_datetime(&local) {
            LocalResult::Single(time) => return Ok(time.timestamp()),
            LocalResult::Ambiguous(first, second) => {
                return Ok(first.timestamp().min(second.timestamp()));
            }
            LocalResult::None => {}
        }
    }
    Err(format!("No valid time near {date} in {zone}"))
}

pub fn format_timestamp(settings: &Settings, timestamp: i64) -> String {
    let zone = settings.time_zone.parse::<Tz>().unwrap_or(chrono_tz::UTC);
    zone.timestamp_opt(timestamp, 0)
        .single()
        .map(|time| {
            let format = if settings.locale.starts_with("en-US") {
                "%m/%d/%Y %I:%M %p %Z"
            } else {
                "%d/%m/%Y %H:%M %Z"
            };
            time.format(format).to_string()
        })
        .unwrap_or_else(|| "Date unavailable".into())
}

/// Explicit clock input differs from a calendar boundary: a nonexistent wall
/// time is an error, while a repeated wall time selects its first occurrence.
pub fn local_timestamp(local: NaiveDateTime, zone: &str) -> Result<i64, String> {
    let zone: Tz = zone
        .parse()
        .map_err(|_| format!("Unknown time zone: {zone}"))?;
    match zone.from_local_datetime(&local) {
        LocalResult::Single(time) => Ok(time.timestamp()),
        LocalResult::Ambiguous(first, second) => Ok(first.timestamp().min(second.timestamp())),
        LocalResult::None => Err(format!(
            "The time {local} does not exist in {zone}. Choose another time."
        )),
    }
}

/// None denotes a named preset. Explicit numeric input is always validated.
pub fn explicit_when(value: &str, zone: &str, locale: &str) -> Result<Option<i64>, String> {
    let value = value.trim();
    // Queued actions retain an absolute instant across restarts and zone changes.
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(Some(time.timestamp()));
    }
    for format in ["%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(local) = NaiveDateTime::parse_from_str(value, format) {
            return local_timestamp(local, zone).map(Some);
        }
    }
    let regional = if locale.eq_ignore_ascii_case("en-US") {
        "%m/%d/%Y"
    } else {
        "%d/%m/%Y"
    };
    for format in ["%Y-%m-%d", "%Y/%m/%d", regional] {
        if let Ok(date) = NaiveDate::parse_from_str(value, format) {
            return local_timestamp(date.and_hms_opt(9, 0, 0).ok_or("Date out of range")?, zone)
                .map(Some);
        }
    }
    if value.starts_with(|c: char| c.is_ascii_digit()) {
        Err("Check the complete date and time (YYYY-MM-DDTHH:mm)".into())
    } else {
        Ok(None)
    }
}

/// Resolve both explicit input and presets against the caller's clock and zone.
pub fn parse_when(value: &str, now: i64, zone: &str, locale: &str) -> Result<i64, String> {
    if let Some(time) = explicit_when(value, zone, locale)? {
        return Ok(time);
    }
    let zone: Tz = zone.parse().map_err(|_| format!("Unknown time zone: {zone}"))?;
    let now = zone.timestamp_opt(now, 0).single().ok_or("Date out of range")?;
    let normalized = value.trim().replace([' ', '_', '-'], "");
    let kind = crate::rules::ScheduleKind::Snooze;
    crate::rules::Preset::list(kind)
        .into_iter()
        .chain(crate::rules::Preset::more(kind))
        .find(|preset| preset.label().replace(' ', "").eq_ignore_ascii_case(&normalized))
        .and_then(|preset| preset.time(now))
        .ok_or_else(|| "Choose a valid date or preset".into())
}

pub fn set_email_date(
    store: &mut crate::model::Store,
    id: u64,
    action: &str,
    when: i64,
    now: i64,
) -> Result<String, String> {
    if action == "changeReminder"
        && when > now.saturating_add(crate::rules::MAX_DAYS_REMINDER * 86_400)
    {
        return Err(format!(
            "A reminder can be at most {} days ahead.",
            crate::rules::MAX_DAYS_REMINDER
        ));
    }
    let email = store
        .email_mut(id)
        .ok_or_else(|| format!("No email with id {id}"))?;
    match action {
        "snooze" => email.snoozed_until = Some(when),
        "changeReminder" => email.reminder_at = Some(when),
        "setDue" => email.due = Some(when),
        _ => return Err("Unsupported date action".into()),
    }
    Ok(format!("{action} done on {id}"))
}

#[cfg(test)]
mod regional_calendar_tests {
    use super::*;
    fn local(value: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").unwrap()
    }
    #[test]
    fn explicit_gap_is_an_error_and_repeated_clock_selects_first_occurrence() {
        assert!(local_timestamp(local("2026-03-29T02:30"), "Europe/Rome").is_err());
        let first = local_timestamp(local("2026-10-25T02:30"), "Europe/Rome").unwrap();
        assert_eq!(
            chrono::DateTime::from_timestamp(first, 0)
                .unwrap()
                .to_rfc3339(),
            "2026-10-25T00:30:00+00:00"
        );
        assert_eq!(
            explicit_when("2026-10-25T02:30", "Europe/Rome", "it-IT").unwrap(),
            Some(first)
        );
        assert!(explicit_when("2026-03-29T02:30", "Europe/Rome", "it-IT").is_err());
    }
}

/// A count with thousands separators: `48.300` in Italian, `48,300` otherwise.
pub fn group_digits(value: u64, italian: bool) -> String {
    let digits = value.to_string();
    let separator = if italian { '.' } else { ',' };
    let mut output = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            output.push(separator);
        }
        output.push(digit);
    }
    output
}

#[cfg(test)]
mod group_digits_tests {
    #[test]
    fn counts_are_grouped_by_thousands() {
        assert_eq!(super::group_digits(48_300, true), "48.300");
        assert_eq!(super::group_digits(1_240, false), "1,240");
        assert_eq!(super::group_digits(999, true), "999");
        assert_eq!(super::group_digits(0, true), "0");
    }
}
