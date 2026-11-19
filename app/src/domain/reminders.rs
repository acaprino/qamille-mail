//! Meeting reminders: which upcoming meetings get a join chip, and how to record
//! them. Pure and deterministic; the caller supplies events, time and devices.
use crate::domain::calendar::google_timestamp;
use crate::domain::meeting::AudioDevice;
use crate::domain::model::{Event, Rsvp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// Hosts recognised in a location or in notes. A subdomain matches its parent.
pub const KNOWN_MEETING_HOSTS: [&str; 8] = [
    "meet.google.com",
    "zoom.us",
    "teams.microsoft.com",
    "teams.live.com",
    "meet.jit.si",
    "webex.com",
    "whereby.com",
    "meet.goto.com",
];
pub const LEAD_MINUTES: [u32; 4] = [1, 2, 5, 10];
pub const DEFAULT_LEAD_MINUTES: u32 = 1;
/// A chip without an answer stays this long after the start, or until the end.
pub const VISIBLE_AFTER_START_SECS: i64 = 600;
pub const MAX_CHIPS: usize = 3;
/// How far past the lead time a Google read looks, so the cached read covers a minute.
pub const LOOKAHEAD_SECS: i64 = 900;

/// One occurrence of an event, normalised from the local calendar or Google.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReminderCandidate {
    /// Origin, event and start: the identity of one occurrence.
    pub key: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub declined: bool,
    pub link: Option<String>,
    /// The local event a recording links to, when one exists.
    pub local_event_id: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "minutes", rename_all = "snake_case")]
pub enum Relative {
    In(i64),
    Now,
    Since(i64),
}

/// What the chip page shows. The link and the linked event stay in the engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReminderChip {
    pub key: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub relative: Relative,
    #[serde(skip)]
    pub link: String,
    #[serde(skip)]
    pub local_event_id: Option<u64>,
}

pub fn valid_lead(minutes: u32) -> bool {
    LEAD_MINUTES.contains(&minutes)
}

pub fn effective_lead(minutes: u32) -> u32 {
    if valid_lead(minutes) { minutes } else { DEFAULT_LEAD_MINUTES }
}

/// The range a reminder check reads, from chips that may still be visible to
/// the furthest start that could appear before the next read.
pub fn read_range(now: i64, lead_minutes: u32) -> (i64, i64) {
    (
        now - VISIBLE_AFTER_START_SECS,
        now + i64::from(effective_lead(lead_minutes)) * 60 + LOOKAHEAD_SECS,
    )
}

pub fn relative(start: i64, now: i64) -> Relative {
    if now < start {
        Relative::In((start - now + 59) / 60)
    } else if now - start < 60 {
        Relative::Now
    } else {
        Relative::Since((now - start) / 60)
    }
}

/// An `https://` link without spaces, control characters or backslashes (which
/// browsers read as `/`, changing the host), with a host.
pub fn valid_link(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| {
        !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '\\')
            && host_of(rest).is_some_and(|h| !h.is_empty())
    })
}

fn host_of(after_scheme: &str) -> Option<String> {
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    Some(host.trim_end_matches('.').to_ascii_lowercase())
}

pub fn known_meeting_host(url: &str) -> bool {
    valid_link(url)
        && url
            .strip_prefix("https://")
        .and_then(host_of)
        .is_some_and(|host| {
            KNOWN_MEETING_HOSTS
                .iter()
                .any(|known| host == *known || host.ends_with(&format!(".{known}")))
        })
}

/// The first `https://` URL in free text whose host is a known meeting service.
pub fn meeting_url_in_text(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(at) = rest.find("https://") {
        let candidate: String = rest[at..]
            .chars()
            .take_while(|c| !c.is_whitespace() && !"\"'<>()[]{}".contains(*c))
            .collect();
        let candidate = candidate.trim_end_matches(['.', ',', ';', ':', '!', '?']);
        if known_meeting_host(candidate) {
            return Some(candidate.to_string());
        }
        rest = &rest[at + "https://".len()..];
    }
    None
}

/// Direct links in order, then a known service in the location, then in the notes.
pub fn meeting_link(direct: &[Option<&str>], location: &str, notes: &str) -> Option<String> {
    direct
        .iter()
        .flatten()
        .map(|link| link.trim())
        .find(|link| valid_link(link))
        .map(String::from)
        .or_else(|| meeting_url_in_text(location))
        .or_else(|| meeting_url_in_text(notes))
}

pub fn local_candidate(event: &Event, all_day: bool) -> ReminderCandidate {
    ReminderCandidate {
        key: format!("local:{}:{}", event.id, event.start),
        title: event.title.clone(),
        start: event.start,
        end: event.end,
        all_day,
        declined: event.rsvp == Rsvp::Declined,
        link: meeting_link(&[event.meeting_link.as_deref()], &event.location, &event.notes),
        local_event_id: Some(event.id),
    }
}

/// A Google Calendar API event, or `None` when it is cancelled or unreadable.
pub fn google_candidate(value: &Value, account: u32, zone: &str) -> Option<ReminderCandidate> {
    if value["status"].as_str() == Some("cancelled") {
        return None;
    }
    let id = value["id"].as_str()?;
    let start = google_timestamp(&value["start"], zone).ok()?;
    let end = google_timestamp(&value["end"], zone).ok()?;
    let declined = value["attendees"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|a| a["self"].as_bool() == Some(true) && a["responseStatus"].as_str() == Some("declined"));
    let video = value["conferenceData"]["entryPoints"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["entryPointType"].as_str() == Some("video"))
        .and_then(|p| p["uri"].as_str());
    Some(ReminderCandidate {
        key: format!("google:{account}:{id}:{start}"),
        // Empty when Google has no title: the chip and the recording use their
        // own localized defaults.
        title: value["summary"].as_str().unwrap_or_default().trim().to_string(),
        start,
        end,
        all_day: value["start"]["date"].is_string(),
        declined,
        link: meeting_link(
            &[value["hangoutLink"].as_str(), video],
            value["location"].as_str().unwrap_or_default(),
            value["description"].as_str().unwrap_or_default(),
        ),
        local_event_id: None,
    })
}

/// The chips to show at `now`: eligible, with a valid link, not yet handled,
/// ordered by start then title, at most three. A meeting found in several
/// places (two accounts, or Google and a local copy) shows once: the copy linked
/// to a local event wins, so a recording links to it, and closing any copy
/// closes the meeting.
pub fn select(
    candidates: &[ReminderCandidate],
    now: i64,
    lead_minutes: u32,
    handled: &HashSet<String>,
) -> Vec<ReminderChip> {
    let lead = i64::from(effective_lead(lead_minutes)) * 60;
    let mut eligible: Vec<_> = candidates
        .iter()
        .filter(|c| !c.all_day && !c.declined && c.end > c.start)
        .filter(|c| c.start - lead <= now && now < (c.start + VISIBLE_AFTER_START_SECS).min(c.end))
        .filter_map(|c| c.link.as_deref().filter(|link| valid_link(link)).map(|link| (c, link)))
        .collect();
    let closed: HashSet<_> = eligible
        .iter()
        .filter(|(c, _)| handled.contains(&c.key))
        .map(|(c, link)| (*link, c.start))
        .collect();
    eligible.sort_by(|(a, _), (b, _)| {
        (a.local_event_id.is_none(), &a.title, &a.key).cmp(&(b.local_event_id.is_none(), &b.title, &b.key))
    });
    let mut seen = HashSet::new();
    eligible.retain(|(c, link)| !closed.contains(&(*link, c.start)) && seen.insert((*link, c.start)));
    eligible.sort_by(|(a, _), (b, _)| (a.start, &a.title, &a.key).cmp(&(b.start, &b.title, &b.key)));
    eligible
        .into_iter()
        .take(MAX_CHIPS)
        .map(|(c, link)| ReminderChip {
            key: c.key.clone(),
            title: c.title.clone(),
            start: c.start,
            end: c.end,
            relative: relative(c.start, now),
            link: link.to_string(),
            local_event_id: c.local_event_id,
        })
        .collect()
}

/// Derived from the occurrence, so a repeated click starts one recording.
pub fn start_request_id(key: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("reminder-{hash:016x}")
}

/// The default microphone, otherwise the first one; the default output as system audio.
pub fn recording_devices(devices: &[AudioDevice]) -> (Option<String>, Option<String>) {
    recording_devices_with(devices, "", "")
}

/// Like `recording_devices`, but a device chosen in Settings is used while it is still there:
/// `microphone` and `system` are device ids, empty to follow the system default. An id that
/// is not in the list (unplugged) or belongs to the other kind falls back to the default.
/// "Registra ora" applies the same rule on the page (`pickDevices`).
pub fn recording_devices_with(
    devices: &[AudioDevice],
    microphone: &str,
    system: &str,
) -> (Option<String>, Option<String>) {
    let microphones: Vec<_> = devices
        .iter()
        .filter(|d| matches!(d.kind.as_str(), "microphone" | "input"))
        .collect();
    let outputs: Vec<_> = devices
        .iter()
        .filter(|d| matches!(d.kind.as_str(), "system" | "output"))
        .collect();
    let chosen = |list: &[&AudioDevice], id: &str| {
        list.iter().find(|d| !id.is_empty() && d.id == id).map(|d| d.id.clone())
    };
    let microphone = chosen(&microphones, microphone).or_else(|| {
        microphones
            .iter()
            .find(|d| d.is_default)
            .or(microphones.first())
            .map(|d| d.id.clone())
    });
    let system = chosen(&outputs, system)
        .or_else(|| outputs.iter().find(|d| d.is_default).map(|d| d.id.clone()));
    (microphone, system)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const T: i64 = 1_793_000_000; // a fixed meeting start

    fn meeting(key: &str, start: i64) -> ReminderCandidate {
        ReminderCandidate {
            key: key.into(),
            title: key.into(),
            start,
            end: start + 1800,
            link: Some(format!("https://meet.google.com/{key}")),
            ..Default::default()
        }
    }
    fn keys(chips: &[ReminderChip]) -> Vec<&str> {
        chips.iter().map(|c| c.key.as_str()).collect()
    }
    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn appears_at_the_lead_time_and_leaves_ten_minutes_after_the_start() {
        let c = [meeting("standup", T)];
        assert!(select(&c, T - 61, 1, &none()).is_empty());
        assert_eq!(keys(&select(&c, T - 60, 1, &none())), ["standup"]);
        assert_eq!(keys(&select(&c, T - 300, 5, &none())), ["standup"]);
        assert_eq!(keys(&select(&c, T + 599, 1, &none())), ["standup"]);
        assert!(select(&c, T + 600, 1, &none()).is_empty());
    }

    #[test]
    fn a_short_meeting_leaves_at_its_end() {
        let mut c = meeting("quick", T);
        c.end = T + 300;
        assert_eq!(keys(&select(&[c.clone()], T + 299, 1, &none())), ["quick"]);
        assert!(select(&[c], T + 300, 1, &none()).is_empty());
    }

    #[test]
    fn an_unknown_lead_time_falls_back_to_one_minute() {
        let c = [meeting("standup", T)];
        assert!(select(&c, T - 120, 3, &none()).is_empty());
        assert_eq!(keys(&select(&c, T - 60, 3, &none())), ["standup"]);
        assert!(valid_lead(10) && !valid_lead(0) && !valid_lead(3));
    }

    #[test]
    fn declined_all_day_and_linkless_events_get_no_chip() {
        let mut declined = meeting("declined", T);
        declined.declined = true;
        let mut all_day = meeting("all-day", T);
        all_day.all_day = true;
        let mut linkless = meeting("linkless", T);
        linkless.link = None;
        assert!(select(&[declined, all_day, linkless], T, 1, &none()).is_empty());
    }

    #[test]
    fn handled_occurrences_stay_hidden() {
        let c = [meeting("standup", T)];
        let handled = HashSet::from(["standup".to_string()]);
        assert!(select(&c, T, 1, &handled).is_empty());
    }

    #[test]
    fn chips_are_ordered_by_start_then_title_and_capped_at_three() {
        let c = [
            meeting("d", T + 30),
            meeting("b", T),
            meeting("a", T),
            meeting("c", T + 10),
            meeting("e", T + 40),
        ];
        assert_eq!(keys(&select(&c, T, 1, &none())), ["a", "b", "c"]);
        let handled = HashSet::from(["a".to_string()]);
        assert_eq!(keys(&select(&c, T, 1, &handled)), ["b", "c", "d"]);
    }

    #[test]
    fn the_same_meeting_in_two_accounts_shows_once() {
        let mut first = meeting("google:1:x", T);
        let mut second = meeting("google:2:x", T);
        first.link = Some("https://meet.google.com/abc".into());
        second.link = first.link.clone();
        let chips = select(&[second, first], T, 1, &none());
        assert_eq!(keys(&chips), ["google:1:x"]);
        assert_eq!(chips[0].link, "https://meet.google.com/abc");
    }

    #[test]
    fn a_local_copy_wins_over_the_same_google_meeting() {
        let mut google = meeting("google:1:x", T);
        let mut local = meeting("local:9:x", T);
        google.link = Some("https://meet.google.com/abc".into());
        local.link = google.link.clone();
        local.local_event_id = Some(9);
        let chips = select(&[google.clone(), local.clone()], T, 1, &none());
        assert_eq!(keys(&chips), ["local:9:x"], "the recording links to the local event");
        assert_eq!(chips[0].local_event_id, Some(9));

        let closed = HashSet::from(["google:1:x".to_string()]);
        assert!(select(&[google, local], T, 1, &closed).is_empty(), "closing one copy closes the meeting");
    }

    #[test]
    fn a_candidate_with_an_unsafe_link_gets_no_chip() {
        let mut unsafe_link = meeting("a", T);
        unsafe_link.link = Some("http://meet.google.com/a".into());
        assert!(select(&[unsafe_link], T, 1, &none()).is_empty());
    }

    #[test]
    fn relative_time_rounds_up_before_and_down_after_the_start() {
        assert_eq!(relative(T, T - 60), Relative::In(1));
        assert_eq!(relative(T, T - 61), Relative::In(2));
        assert_eq!(relative(T, T - 1), Relative::In(1));
        assert_eq!(relative(T, T), Relative::Now);
        assert_eq!(relative(T, T + 59), Relative::Now);
        assert_eq!(relative(T, T + 240), Relative::Since(4));
        assert_eq!(relative(T, T + 299), Relative::Since(4));
        assert_eq!(
            serde_json::to_value(Relative::Since(4)).unwrap(),
            json!({"kind":"since","minutes":4})
        );
    }

    #[test]
    fn links_must_be_https_without_spaces() {
        assert!(valid_link("https://acme.zoom.us/j/1"));
        for bad in ["http://zoom.us/j/1", "https://", "https://zoom.us/j 1", "javascript:alert(1)", "https://zoom.us/\n"] {
            assert!(!valid_link(bad), "{bad:?}");
        }
    }

    #[test]
    fn known_hosts_match_domains_and_subdomains_only() {
        assert!(known_meeting_host("https://acme.zoom.us/j/1"));
        assert!(known_meeting_host("https://ZOOM.US/j/1"));
        assert!(known_meeting_host("https://meet.google.com/abc-defg-hij"));
        assert!(known_meeting_host("https://user@acme.webex.com:443/meet"));
        assert!(!known_meeting_host("https://notzoom.us/j/1"));
        assert!(!known_meeting_host("https://zoom.us.example.com/j/1"));
        assert!(!known_meeting_host("https://example.com/?u=https://zoom.us"));
        // Browsers treat a backslash as `/`, so the real host would be evil.example.
        assert!(!known_meeting_host("https://evil.example\\@acme.zoom.us/j/123"));
        assert_eq!(meeting_url_in_text("vedi https://evil.example\\@acme.zoom.us/j/123"), None);
    }

    #[test]
    fn link_order_is_direct_then_location_then_notes() {
        assert_eq!(
            meeting_link(&[None, Some("https://custom.example/room")], "https://zoom.us/j/1", ""),
            Some("https://custom.example/room".into())
        );
        assert_eq!(
            meeting_link(&[Some("http://insecure"), None], "Sala 3", "Join: https://acme.zoom.us/j/123."),
            Some("https://acme.zoom.us/j/123".into())
        );
        // EX-12: a link of an unknown service in the location is skipped.
        assert_eq!(
            meeting_link(&[None], "Sala 3, vedi https://example.com/agenda", "https://acme.zoom.us/j/123"),
            Some("https://acme.zoom.us/j/123".into())
        );
        assert_eq!(meeting_link(&[None], "Sala 3", "no link"), None);
    }

    #[test]
    fn google_events_read_conference_links_attendance_and_all_day() {
        let zoom = json!({
            "id": "abc_20261103T090000Z", "summary": "Standup",
            "start": {"dateTime": "2026-11-03T10:00:00+01:00"}, "end": {"dateTime": "2026-11-03T10:15:00+01:00"},
            "conferenceData": {"entryPoints": [
                {"entryPointType": "phone", "uri": "tel:+39-000"},
                {"entryPointType": "video", "uri": "https://acme.zoom.us/j/9"}
            ]},
            "attendees": [{"email": "me@x.it", "self": true, "responseStatus": "accepted"}]
        });
        let c = google_candidate(&zoom, 4, "Europe/Rome").unwrap();
        assert_eq!(c.key, format!("google:4:abc_20261103T090000Z:{}", c.start));
        assert_eq!(c.end - c.start, 900);
        assert_eq!(c.link.as_deref(), Some("https://acme.zoom.us/j/9"));
        assert!(!c.declined && !c.all_day);

        let mut meet = zoom.clone();
        meet["hangoutLink"] = json!("https://meet.google.com/aaa-bbbb-ccc");
        meet["attendees"][0]["responseStatus"] = json!("declined");
        let c = google_candidate(&meet, 4, "Europe/Rome").unwrap();
        assert_eq!(c.link.as_deref(), Some("https://meet.google.com/aaa-bbbb-ccc"));
        assert!(c.declined);

        let all_day = json!({"id": "d", "start": {"date": "2026-11-03"}, "end": {"date": "2026-11-04"}, "hangoutLink": "https://meet.google.com/x"});
        let all_day = google_candidate(&all_day, 4, "Europe/Rome").unwrap();
        assert!(all_day.all_day);
        assert_eq!(all_day.title, "", "no title: the chip and the recording use localized defaults");
        let cancelled = json!({"id": "c", "status": "cancelled"});
        assert!(google_candidate(&cancelled, 4, "Europe/Rome").is_none());
    }

    #[test]
    fn local_events_keep_their_id_for_the_recording() {
        let event = Event {
            id: 42,
            title: "Design review".into(),
            start: T,
            end: T + 3600,
            location: "https://teams.microsoft.com/l/meetup-join/1".into(),
            rsvp: Rsvp::Declined,
            ..Default::default()
        };
        let c = local_candidate(&event, false);
        assert_eq!(c.key, format!("local:42:{T}"));
        assert_eq!(c.local_event_id, Some(42));
        assert_eq!(c.link.as_deref(), Some("https://teams.microsoft.com/l/meetup-join/1"));
        assert!(c.declined);
    }

    #[test]
    fn request_ids_are_stable_short_and_distinct() {
        let a = start_request_id("google:1:abc:1793000000");
        assert_eq!(a, start_request_id("google:1:abc:1793000000"));
        assert_ne!(a, start_request_id("google:1:abc:1793003600"));
        assert!(a.len() <= 200);
    }

    #[test]
    fn recording_uses_default_devices_with_a_microphone_fallback() {
        let device = |id: &str, kind: &str, is_default: bool| AudioDevice {
            id: id.into(),
            name: id.into(),
            kind: kind.into(),
            is_default,
        };
        let all = [
            device("mic-1", "microphone", false),
            device("mic-2", "microphone", true),
            device("out-1", "system", false),
            device("out-2", "system", true),
        ];
        assert_eq!(recording_devices(&all), (Some("mic-2".into()), Some("out-2".into())));
        let no_defaults = [device("mic-1", "microphone", false), device("out-1", "system", false)];
        assert_eq!(recording_devices(&no_defaults), (Some("mic-1".into()), None));
        assert_eq!(recording_devices(&[]), (None, None));
    }

    #[test]
    fn recording_prefers_the_devices_chosen_in_settings_while_they_exist() {
        let device = |id: &str, kind: &str, is_default: bool| AudioDevice {
            id: id.into(),
            name: id.into(),
            kind: kind.into(),
            is_default,
        };
        let all = [
            device("mic-1", "input", false),
            device("mic-2", "microphone", true),
            device("out-1", "output", true),
            device("out-2", "system", false),
        ];
        let chosen = recording_devices_with(&all, "mic-1", "out-2");
        assert_eq!(chosen, (Some("mic-1".into()), Some("out-2".into())));
        // Empty follows the system default; an id that is gone, or of the other kind, too.
        assert_eq!(recording_devices_with(&all, "", ""), recording_devices(&all));
        let stale = recording_devices_with(&all, "unplugged", "mic-1");
        assert_eq!(stale, (Some("mic-2".into()), Some("out-1".into())));
        assert_eq!(recording_devices_with(&all, "out-1", "mic-2"), stale);
        // No default output and none chosen: no system audio, as before.
        let no_output_default = [device("mic-1", "microphone", false), device("out-1", "system", false)];
        assert_eq!(recording_devices_with(&no_output_default, "", "out-1"), (Some("mic-1".into()), Some("out-1".into())));
        assert_eq!(recording_devices_with(&no_output_default, "", ""), (Some("mic-1".into()), None));
    }

    #[test]
    fn chip_json_never_carries_the_link() {
        let chips = select(&[meeting("standup", T)], T, 1, &none());
        let value = serde_json::to_value(&chips[0]).unwrap();
        assert!(value.get("link").is_none() && value.get("local_event_id").is_none());
    }
}
