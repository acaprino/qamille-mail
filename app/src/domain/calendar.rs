//! Provider-independent calendar helpers. Dates use half-open intervals.
#[derive(Debug)]
pub struct CalendarError(pub String);
impl CalendarError {
    pub fn validation(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}
impl std::fmt::Display for CalendarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CalendarError {}
pub type CalendarResult<T> = Result<T, CalendarError>;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CalendarDescriptor {
    pub id: String,
    pub provider_id: String,
    pub account_id: Option<u32>,
    pub name: String,
    pub time_zone: String,
    pub writable: bool,
    pub supports_recurrence: bool,
}

pub fn timestamp(value: &str, zone: &str) -> CalendarResult<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(dt.timestamp());
    }
    let day = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        CalendarError::validation("Use an RFC3339 date-time with offset, or YYYY-MM-DD for a date")
    })?;
    crate::domain::regional::day_start(day, zone).map_err(CalendarError::validation)
}

pub fn google_time(value: &str, zone: &str, all_day: bool) -> CalendarResult<Value> {
    timestamp(value, zone)?;
    if all_day {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
            CalendarError::validation(
                "All-day event boundaries must use YYYY-MM-DD; end is exclusive",
            )
        })?;
        Ok(json!({"date": value}))
    } else {
        DateTime::parse_from_rfc3339(value).map_err(|_| {
            CalendarError::validation("Timed events require RFC3339 with an explicit offset")
        })?;
        Ok(json!({"dateTime": value, "timeZone": zone}))
    }
}

pub fn google_timestamp(value: &Value, zone: &str) -> CalendarResult<i64> {
    timestamp(
        value["dateTime"]
            .as_str()
            .or_else(|| value["date"].as_str())
            .ok_or_else(|| CalendarError::validation("Calendar event has no date"))?,
        value["timeZone"].as_str().unwrap_or(zone),
    )
}

pub fn rfc3339(ts: i64) -> CalendarResult<String> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .map(|v| v.to_rfc3339())
        .ok_or_else(|| CalendarError::validation("Calendar date is out of range"))
}

pub fn free_slots(
    mut busy: Vec<(i64, i64)>,
    from: i64,
    to: i64,
    minutes: u32,
    buffer_minutes: u32,
) -> CalendarResult<Vec<Value>> {
    if to <= from || minutes == 0 || minutes > 1440 {
        return Err(CalendarError::validation(
            "Choose an increasing range and a duration from 1 to 1440 minutes",
        ));
    }
    let duration = i64::from(minutes) * 60;
    let buffer = i64::from(buffer_minutes) * 60;
    busy.sort_unstable();
    let mut next = from;
    let mut slots = Vec::new();
    for (start, end) in busy {
        let start = start.saturating_sub(buffer).max(from);
        let end = end.saturating_add(buffer).min(to);
        if end <= from || start >= to {
            continue;
        }
        if start.saturating_sub(next) >= duration {
            slots.push(json!({"start": rfc3339(next)?, "end": rfc3339(start)?}));
        }
        next = next.max(end);
    }
    if to.saturating_sub(next) >= duration {
        slots.push(json!({"start": rfc3339(next)?, "end": rfc3339(to)?}));
    }
    Ok(slots)
}

/// Split a range into civil working days before subtracting merged busy time.
/// Weekday numbers follow ISO 8601: Monday=1 ... Sunday=7.
pub fn working_slots(
    busy: Vec<(i64, i64)>,
    from: i64,
    to: i64,
    zone: &str,
    work_start: &str,
    work_end: &str,
    weekdays: &[u32],
    minutes: u32,
    buffer_minutes: u32,
) -> CalendarResult<Vec<Value>> {
    use chrono::{Datelike, NaiveTime};
    let z = zone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| CalendarError::validation("Choose an IANA time zone"))?;
    let start = NaiveTime::parse_from_str(work_start, "%H:%M")
        .map_err(|_| CalendarError::validation("work_start must use HH:MM"))?;
    let end = NaiveTime::parse_from_str(work_end, "%H:%M")
        .map_err(|_| CalendarError::validation("work_end must use HH:MM"))?;
    if end <= start
        || to <= from
        || to.saturating_sub(from) > 366 * 86400
        || weekdays.iter().any(|d| !(1..=7).contains(d))
        || minutes == 0
        || minutes > 1440
        || buffer_minutes > 1440
    {
        return Err(CalendarError::validation(
            "Choose increasing working hours, a range of at most 366 days, weekdays 1–7, and valid duration/margins",
        ));
    }
    let mut day = z
        .timestamp_opt(from, 0)
        .single()
        .ok_or_else(|| CalendarError::validation("Range start out of bounds"))?
        .date_naive();
    let last = z
        .timestamp_opt(to - 1, 0)
        .single()
        .ok_or_else(|| CalendarError::validation("Range end out of bounds"))?
        .date_naive();
    let mut result = Vec::new();
    while day <= last {
        if weekdays.contains(&day.weekday().number_from_monday()) {
            let a = crate::domain::regional::local_timestamp(day.and_time(start), zone)
                .map_err(CalendarError::validation)?
                .max(from);
            let b = crate::domain::regional::local_timestamp(day.and_time(end), zone)
                .map_err(CalendarError::validation)?
                .min(to);
            if b > a {
                result.extend(free_slots(busy.clone(), a, b, minutes, buffer_minutes)?);
            }
        }
        day = day
            .succ_opt()
            .ok_or_else(|| CalendarError::validation("Range end out of bounds"))?;
    }
    Ok(result)
}

pub fn escape_ics(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\r', "")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

pub fn export_ics(events: &[Value], zone: &str) -> CalendarResult<String> {
    let mut out = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Qamille//Calendar//EN\r\nCALSCALE:GREGORIAN\r\n",
    );
    for e in events {
        out.push_str("BEGIN:VEVENT\r\n");
        let uid = e["iCalUID"]
            .as_str()
            .or_else(|| e["id"].as_str())
            .ok_or_else(|| CalendarError::validation("Event ID missing"))?;
        let stamp = e["updated"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ").to_string())
            .unwrap_or_else(|| "19700101T000000Z".into());
        out.push_str(&format!(
            "UID:{}\r\nDTSTAMP:{stamp}\r\nSUMMARY:{}\r\n",
            escape_ics(uid),
            escape_ics(e["summary"].as_str().unwrap_or(""))
        ));
        for (name, key) in [("DTSTART", "start"), ("DTEND", "end")] {
            if let Some(date) = e[key]["date"].as_str() {
                out.push_str(&format!("{name};VALUE=DATE:{}\r\n", date.replace('-', "")));
            } else {
                let ts = google_timestamp(&e[key], zone)?;
                let time_zone = e[key]["timeZone"].as_str().unwrap_or(zone);
                let time_zone = time_zone
                    .parse::<chrono_tz::Tz>()
                    .map_err(|_| CalendarError::validation("Event time zone is invalid"))?;
                let stamp = time_zone
                    .timestamp_opt(ts, 0)
                    .single()
                    .unwrap()
                    .format("%Y%m%dT%H%M%S");
                out.push_str(&format!("{name};TZID={time_zone}:{stamp}\r\n"));
            }
        }
        for (key, name) in [("description", "DESCRIPTION"), ("location", "LOCATION")] {
            if let Some(value) = e[key].as_str() {
                out.push_str(&format!("{name}:{}\r\n", escape_ics(value)));
            }
        }
        if let Some(rules) = e["recurrence"].as_array() {
            for rule in rules.iter().filter_map(Value::as_str) {
                if !rule.contains(['\r', '\n']) {
                    out.push_str(rule);
                    out.push_str("\r\n");
                }
            }
        }
        out.push_str("END:VEVENT\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    Ok(fold_ics(&out))
}

fn fold_ics(input: &str) -> String {
    let mut out = String::new();
    for line in input.split_terminator("\r\n") {
        let mut octets = 0;
        for ch in line.chars() {
            if octets + ch.len_utf8() > 75 {
                out.push_str("\r\n ");
                octets = 1;
            }
            out.push(ch);
            octets += ch.len_utf8();
        }
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_busy_intervals_are_merged_and_buffers_apply() {
        let result = free_slots(vec![(1000, 2000), (1500, 2500)], 0, 4000, 10, 1).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[1]["start"], rfc3339(2560).unwrap());
    }
    #[test]
    fn all_day_dates_keep_civil_date_on_dst_boundary() {
        let a = google_time("2026-10-25", "Europe/Rome", true).unwrap();
        let b = google_time("2026-10-26", "Europe/Rome", true).unwrap();
        assert_eq!(
            google_timestamp(&b, "Europe/Rome").unwrap()
                - google_timestamp(&a, "Europe/Rome").unwrap(),
            25 * 3600
        );
    }
    #[test]
    fn timed_inputs_need_offsets() {
        assert!(google_time("2026-10-25", "Europe/Rome", false).is_err());
    }
    #[test]
    fn ics_values_cannot_inject_properties() {
        assert_eq!(
            escape_ics("x\r\nATTENDEE:evil,yes"),
            "x\\nATTENDEE:evil\\,yes"
        );
    }
    #[test]
    fn ics_lines_fold_at_utf8_boundaries() {
        let text = format!("SUMMARY:{}\r\n", "è".repeat(100));
        let folded = fold_ics(&text);
        assert!(folded.split_terminator("\r\n").all(|s| s.len() <= 75));
        assert_eq!(folded.replace("\r\n ", ""), text);
    }
    #[test]
    fn workdays_and_dst_preserve_civil_work_hours() {
        let from = timestamp("2026-10-23", "Europe/Rome").unwrap();
        let to = timestamp("2026-10-27", "Europe/Rome").unwrap();
        let slots = working_slots(
            vec![],
            from,
            to,
            "Europe/Rome",
            "09:00",
            "17:00",
            &[1, 2, 3, 4, 5],
            30,
            0,
        )
        .unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0]["start"], "2026-10-23T07:00:00+00:00");
        assert_eq!(slots[1]["start"], "2026-10-26T08:00:00+00:00");
    }
    #[test]
    fn event_buffer_extends_into_working_hours() {
        let slots = free_slots(vec![(0, 100)], 200, 4000, 1, 5).unwrap();
        assert_eq!(slots[0]["start"], rfc3339(400).unwrap());
    }
}
