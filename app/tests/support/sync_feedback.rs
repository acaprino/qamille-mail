use super::*;
use qamille::application::mail_sync_status::SyncPhase;

#[test]
fn sync_feedback_is_visible_before_download_and_shared_with_background_clones() {
    let gateway = Arc::new(TestMail::default());
    let mut store = fixture();
    let account_id = store.accounts[0].id;
    store.accounts[0].last_sync = None;
    let ctx = context(store, gateway.clone());
    let mail = MailService::new(ctx);
    let visible = mail.clone();
    *gateway.during_sync.lock().unwrap() = Some(Box::new(move || {
        let state = visible.sync_status();
        assert_eq!(state.accounts.len(), 1);
        assert_eq!(state.accounts[0].account_id, account_id);
        assert_eq!(state.accounts[0].phase, SyncPhase::Syncing);
    }));
    mail.clone().sync_account(account_id).unwrap();
    let done = mail.sync_status();
    assert_eq!(done.accounts[0].phase, SyncPhase::Complete);
    assert!(done.accounts[0].errors.is_empty());
    assert_eq!(done.revision, 2);
}

#[test]
fn failed_first_sync_stops_the_spinner_and_retry_clears_the_error() {
    let gateway = Arc::new(TestMail::default());
    *gateway.sync_failure.lock().unwrap() = Some("Synthetic connection failure".into());
    let mut store = fixture();
    let account_id = store.accounts[0].id;
    store.accounts[0].last_sync = None;
    let ctx = context(store, gateway.clone());
    let mail = MailService::new(ctx.clone());
    let report = mail.sync_account(account_id).unwrap();
    let failed = mail.sync_status();
    assert_eq!(failed.accounts[0].phase, SyncPhase::Error);
    assert_eq!(failed.accounts[0].errors, report.errors);
    assert!(ctx.read().unwrap().account(account_id).unwrap().last_sync.is_none());
    *gateway.sync_failure.lock().unwrap() = None;
    mail.sync_account(account_id).unwrap();
    let retried = mail.sync_status();
    assert_eq!(retried.accounts[0].phase, SyncPhase::Complete);
    assert!(retried.accounts[0].errors.is_empty());
    assert!(retried.revision > failed.revision);
    assert!(ctx.read().unwrap().account(account_id).unwrap().last_sync.is_some());
}

/// The UI pulls `mail_sync_status` when a typed `sync-progress` arrives (the legacy
/// empty `mail-sync-changed` is gone), so the snapshot must already hold the phase and
/// the errors the event announces.
#[test]
fn snapshot_already_holds_what_each_typed_sync_event_announces() {
    use qamille::application::events::AppEvent;
    struct Probe {
        mail: Mutex<Option<MailService>>,
        seen: Mutex<Vec<(SyncPhase, SyncPhase, usize)>>,
    }
    impl EventSink for Probe {
        fn emit(&self, event: AppEvent) {
            let AppEvent::SyncProgress { phase, .. } = event else { return };
            let mail = self.mail.lock().unwrap();
            let status = mail.as_ref().expect("probe wired before the sync").sync_status();
            let held = status.accounts.first().expect("a status for the account");
            self.seen.lock().unwrap().push((phase, held.phase, held.errors.len()));
        }
    }
    let gateway = Arc::new(TestMail::default());
    *gateway.sync_failure.lock().unwrap() = Some("Synthetic connection failure".into());
    let mut store = fixture();
    let account_id = store.accounts[0].id;
    store.accounts[0].last_sync = None;
    let base = context(store, gateway);
    let probe = Arc::new(Probe { mail: Mutex::new(None), seen: Mutex::new(Vec::new()) });
    let ctx = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: probe.clone(),
        repository: base.repository.clone(),
        mail: base.mail.clone(),
        secrets: base.secrets.clone(),
        clock: base.clock.clone(),
        attachments: base.attachments.clone(),
    });
    let mail = MailService::new(ctx);
    *probe.mail.lock().unwrap() = Some(mail.clone());
    mail.sync_account(account_id).unwrap();
    let seen = probe.seen.lock().unwrap().clone();
    assert_eq!(
        seen.iter().map(|(event, held, _)| (*event, *held)).collect::<Vec<_>>(),
        vec![(SyncPhase::Syncing, SyncPhase::Syncing), (SyncPhase::Error, SyncPhase::Error)],
    );
    assert_eq!(seen[0].2, 0);
    assert!(seen[1].2 > 0, "the error text is in the snapshot when the error event goes out");
}