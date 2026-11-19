use chrono::{DateTime, TimeZone};
use qamille::{
    application::{AppResult, ServiceContext, drafts::DraftService, mail::MailService, ports::*, undo::UndoService},
    domain::{regional, undo::{UndoOperation, project}},
    infrastructure::repository::MemoryRepository,
    model::{Account, Attachment, Email, Store},
};
use std::sync::{Arc, atomic::{AtomicI64, Ordering}};

struct TestClock(AtomicI64);
impl Clock for TestClock { fn now(&self) -> i64 { self.0.load(Ordering::SeqCst) } }
struct Unused;
impl SecretStore for Unused {
    fn get(&self, _: &str) -> AppResult<String> { Ok(String::new()) }
    fn set(&self, _: &str, _: &str) -> AppResult<()> { unreachable!() }
}
impl AttachmentGateway for Unused {
    fn total_size(&self, _: &[String]) -> AppResult<u64> { Ok(0) }
    fn read_text(&self, _: &Attachment) -> AppResult<String> { unreachable!() }
}
impl MailGateway for Unused {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> { unreachable!() }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> { unreachable!() }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> { unreachable!() }
    fn check_login(&self, _: &Account) -> AppResult<()> { unreachable!() }
}
fn timestamp(value: &str) -> i64 { DateTime::parse_from_rfc3339(value).unwrap().timestamp() }
fn context(store: Store, clock: Arc<TestClock>) -> Arc<ServiceContext> {
    Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()), events: Arc::new(NullSink), repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(Unused), secrets: Arc::new(Unused), attachments: Arc::new(Unused), clock })
}
fn fixture(zone: &str, now: &str) -> (Arc<ServiceContext>, Arc<TestClock>, UndoService) {
    let mut store = Store::default();
    store.next_id = 2;
    store.settings.time_zone = zone.into();
    store.settings.locale = "it-IT".into();
    store.emails.push(Email { id: 1, ..Default::default() });
    let clock = Arc::new(TestClock(AtomicI64::new(timestamp(now))));
    let context = context(store, clock.clone());
    let undo = UndoService::new(context.clone(), MailService::new(context.clone()), DraftService::new(context.clone()));
    (context, clock, undo)
}

#[test]
fn queued_snooze_keeps_honolulu_instant_after_restart_and_zone_change() {
    let (original, clock, undo) = fixture("Pacific/Honolulu", "2027-01-14T12:00:00Z");
    let pending = undo.queue_mail(1, "snooze".into(), Some("2027-01-15T09:00".into())).unwrap();
    let expected = timestamp("2027-01-15T19:00:00Z");
    let UndoOperation::Mail { value: Some(value), .. } = pending.operation else { panic!("missing date") };
    assert_eq!(timestamp(&value), expected);
    assert_eq!(project(original.read().unwrap()).email(1).unwrap().snoozed_until, Some(expected));
    // Deserialize the persisted queue and change regional preferences before commit.
    let mut saved: Store = serde_json::from_slice(&serde_json::to_vec(&original.read().unwrap()).unwrap()).unwrap();
    saved.settings.time_zone = "Europe/Rome".into();
    let restarted = context(saved, clock.clone());
    let undo = UndoService::new(restarted.clone(), MailService::new(restarted.clone()), DraftService::new(restarted.clone()));
    clock.0.fetch_add(31, Ordering::SeqCst);
    undo.recover_interrupted().unwrap();
    assert!(undo.process_due().unwrap());
    assert_eq!(restarted.read().unwrap().email(1).unwrap().snoozed_until, Some(expected));
    assert!(restarted.read().unwrap().undo_actions.is_empty());
}

#[test]
fn tomorrow_uses_application_clock_and_civil_day_across_dst() {
    for (now, tomorrow) in [
        ("2026-03-28T23:30:00+01:00", "2026-03-29T09:00:00+02:00"),
        ("2026-10-25T00:30:00+02:00", "2026-10-26T09:00:00+01:00"),
    ] {
        let (ctx, _, undo) = fixture("Europe/Rome", now);
        undo.queue_mail(1, "snooze".into(), Some("Tomorrow".into())).unwrap();
        assert_eq!(project(ctx.read().unwrap()).email(1).unwrap().snoozed_until, Some(timestamp(tomorrow)));
    }
    let (ctx, _, undo) = fixture("Pacific/Honolulu", "2027-01-15T08:30:00Z");
    undo.queue_mail(1, "snooze".into(), Some("Tomorrow".into())).unwrap();
    assert_eq!(project(ctx.read().unwrap()).email(1).unwrap().snoozed_until, Some(timestamp("2027-01-15T19:00:00Z")));
}

#[test]
fn direct_mail_dates_use_same_clock_zone_and_reminder_limit() {
    let (ctx, _, _) = fixture("Pacific/Honolulu", "2027-01-15T08:30:00Z");
    let mail = MailService::new(ctx.clone());
    mail.act(1, "setDue", Some("Tomorrow")).unwrap();
    assert_eq!(ctx.read().unwrap().email(1).unwrap().due, Some(timestamp("2027-01-15T19:00:00Z")));
    assert!(mail.act(1, "changeReminder", Some("2027-04-01T09:00")).is_err());
    assert!(ctx.read().unwrap().email(1).unwrap().reminder_at.is_none());
}

#[test]
fn queued_reminder_projection_uses_the_clock_that_validated_it() {
    let (ctx, _, undo) = fixture("Pacific/Honolulu", "2027-01-15T08:30:00Z");
    undo.queue_mail(1, "changeReminder".into(), Some("Tomorrow".into())).unwrap();
    assert_eq!(project(ctx.read().unwrap()).email(1).unwrap().reminder_at,
        Some(timestamp("2027-01-15T19:00:00Z")));
    assert!(undo.queue_mail(1, "changeReminder".into(), Some("2027-04-01T09:00".into())).is_err());
}

#[test]
fn invalid_dst_time_is_rejected_and_repeated_time_has_stable_offset() {
    let (ctx, _, undo) = fixture("Europe/Rome", "2026-03-28T12:00:00Z");
    assert!(undo.queue_mail(1, "snooze".into(), Some("2026-03-29T02:30".into())).is_err());
    assert!(ctx.read().unwrap().undo_actions.is_empty());
    undo.queue_mail(1, "snooze".into(), Some("2026-10-25T02:30".into())).unwrap();
    assert_eq!(project(ctx.read().unwrap()).email(1).unwrap().snoozed_until, Some(timestamp("2026-10-25T00:30:00Z")));
}

#[test]
fn presets_and_explicit_dates_obey_local_calendar_and_locale() {
    let now = timestamp("2026-03-28T23:30:00+01:00");
    for (preset, expected) in [("Weekend", "2026-03-29T09:00:00+02:00"),
        ("Next Week", "2026-03-30T09:00:00+02:00"), ("Tomorrow Evening", "2026-03-29T20:00:00+02:00")] {
        assert_eq!(regional::parse_when(preset, now, "Europe/Rome", "it-IT").unwrap(), timestamp(expected));
    }
    let us = regional::parse_when("04/05/2027", now, "Pacific/Honolulu", "en-US").unwrap();
    let it = regional::parse_when("04/05/2027", now, "Pacific/Honolulu", "it-IT").unwrap();
    assert_eq!(chrono_tz::Pacific::Honolulu.timestamp_opt(us, 0).unwrap().format("%F").to_string(), "2027-04-05");
    assert_eq!(chrono_tz::Pacific::Honolulu.timestamp_opt(it, 0).unwrap().format("%F").to_string(), "2027-05-04");
    let late = timestamp("2026-11-03T23:55:00+05:45");
    assert_eq!(regional::parse_when("Later Today", late, "Asia/Kathmandu", "en-US").unwrap(),
        timestamp("2026-11-04T00:00:00+05:45"));
}
