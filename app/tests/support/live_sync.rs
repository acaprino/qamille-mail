use super::*;
use crate::application::{
    AppError,
    account_worker::{CommitOutcome, WorkerRegistry},
    drafts::DraftService,
    mail::MailService,
    ports::*,
};
use crate::infrastructure::repository::MemoryRepository;
use crate::model::{Account, AccountKind, Attachment, Email, sample_store};
use std::collections::{HashMap, VecDeque};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Secrets(Mutex<HashMap<String, String>>);
impl SecretStore for Secrets {
    fn get(&self, key: &str) -> AppResult<String> {
        Ok(self.0.lock().unwrap().get(key).cloned().unwrap_or_default())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
}
struct Clock;
impl super::super::ports::Clock for Clock {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
struct Attachments;
impl AttachmentGateway for Attachments {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}
#[derive(Default)]
struct Mail {
    streams: Mutex<HashMap<u32, VecDeque<mpsc::Receiver<AppResult<bool>>>>>,
    incoming: Mutex<Vec<Email>>,
    watched: Mutex<Vec<(u32, String)>>,
    fail_sync: Mutex<Vec<u32>>,
    downloads: AtomicUsize,
    active: Arc<AtomicUsize>,
}
struct Watch {
    rx: mpsc::Receiver<AppResult<bool>>,
    active: Arc<AtomicUsize>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl MailWatch for Watch {
    fn wait(&mut self, timeout: Duration) -> AppResult<bool> {
        match self.rx.recv_timeout(timeout) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(_) => Err(AppError::external("Connection lost")),
        }
    }
}
impl Mail {
    fn stream(&self, id: u32) -> mpsc::Sender<AppResult<bool>> {
        let (tx, rx) = mpsc::channel();
        self.streams
            .lock()
            .unwrap()
            .entry(id)
            .or_default()
            .push_back(rx);
        tx
    }
}
impl MailGateway for Mail {
    fn watch(&self, a: &Account) -> AppResult<Box<dyn MailWatch>> {
        self.watched
            .lock()
            .unwrap()
            .push((a.id, a.password.clone()));
        let rx = self
            .streams
            .lock()
            .unwrap()
            .entry(a.id)
            .or_default()
            .pop_front()
            .ok_or_else(|| AppError::external("Offline"))?;
        self.active.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Watch {
            rx,
            active: self.active.clone(),
        }))
    }
    fn sync(&self, a: &Account) -> AppResult<SyncBatch> {
        if self.fail_sync.lock().unwrap().contains(&a.id) {
            return Err(AppError::external("Provider down"));
        }
        self.downloads.fetch_add(1, Ordering::SeqCst);
        Ok(SyncBatch {
            emails: self
                .incoming
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.account_id == a.id)
                .cloned()
                .collect(),
            archive_folder: None,
            trash_folder: None,
        })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        panic!("Notifications must not mutate Gmail")
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        panic!("Notifications must not send mail")
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}
fn setup(mail: Arc<Mail>) -> Arc<ServiceContext> {
    let mut store = sample_store();
    store.accounts[0].kind = AccountKind::Gmail;
    let secrets = Arc::new(Secrets::default());
    secrets
        .set(&account_key(&store.accounts[0].email), "first-token")
        .unwrap();
    Arc::new(ServiceContext { compute: std::sync::Arc::new(crate::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail,
        secrets,
        clock: Arc::new(Clock),
        attachments: Arc::new(Attachments),
    })
}
/// The dispatch tests need a registry; no confirmation is ever routed through
/// it, so the outcome sink stays inert.
struct NoopOutcome;
impl CommitOutcome for NoopOutcome {
    fn commit_finished(&self, _: &[u64], _: &AppResult<()>) {}
}
fn registry(context: &Arc<ServiceContext>) -> Arc<WorkerRegistry> {
    Arc::new(WorkerRegistry::new(
        MailService::new(context.clone()),
        DraftService::new(context.clone()),
        context.clone(),
        Arc::new(NullSink),
        Arc::new(NoopOutcome),
    ))
}
fn coordinator(context: Arc<ServiceContext>) -> MailSyncService {
    MailSyncService::with_timing(
        context.clone(),
        registry(&context),
        Timing {
            refresh_accounts: Duration::from_millis(10),
            fallback: Duration::from_secs(60),
            wait: Duration::from_millis(5),
            retry_min: Duration::from_millis(10),
            retry_max: Duration::from_millis(40),
            session_lifetime: Duration::from_secs(30),
        },
    )
}
fn until(sync: &mut MailSyncService, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Notification did not reach the mailbox"
        );
        sync.step(Duration::from_millis(5)).unwrap();
    }
}
/// The downloads dispatched to the workers are asynchronous: wait until the
/// counter is unchanged across five consecutive observations, stepping in
/// between, and return the settled value.
fn settled_downloads(sync: &mut MailSyncService, gateway: &Mail) -> usize {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stable = 0;
    let mut last = gateway.downloads.load(Ordering::SeqCst);
    while stable < 5 {
        assert!(Instant::now() < deadline, "Downloads never settled");
        std::thread::sleep(Duration::from_millis(10));
        sync.step(Duration::from_millis(5)).unwrap();
        let current = gateway.downloads.load(Ordering::SeqCst);
        stable = if current == last { stable + 1 } else { 0 };
        last = current;
    }
    last
}
fn incoming(account_id: u32) -> Email {
    Email {
        account_id,
        message_id: format!("new-{account_id}@example.com"),
        subject: "Arrived through IDLE".into(),
        folder: "Inbox".into(),
        uid: Some(901),
        ..Default::default()
    }
}

#[test]
fn new_account_syncs_even_when_its_idle_connection_is_unavailable() {
    let gateway = Arc::new(Mail::default());
    let context = setup(gateway.clone());
    let mut sync = coordinator(context.clone());
    sync.step(Duration::ZERO).unwrap();
    context.update(|store| {
        let mut account = store.accounts[0].clone();
        account.id = 3;
        account.email = "new@example.test".into();
        account.last_sync = None;
        store.accounts.push(account);
        Ok(())
    }).unwrap();
    context.secrets.set(&account_key("new@example.test"), "test-token").unwrap();
    gateway.incoming.lock().unwrap().push(incoming(3));
    // No IDLE stream is available and periodic recovery is still 60 seconds away.
    until(&mut sync, || context.read().unwrap().emails.iter().any(|email| email.message_id == "new-3@example.com"));
    assert!(context.read().unwrap().account(3).unwrap().last_sync.is_some());
}

#[test]
fn notification_updates_mail_without_waiting_for_periodic_sync() {
    let gateway = Arc::new(Mail::default());
    let tx = gateway.stream(1);
    let context = setup(gateway.clone());
    let mut sync = coordinator(context.clone());
    until(&mut sync, || gateway.active.load(Ordering::SeqCst) == 1);
    // Drain the initial catch-up — dispatched to the account worker, so wait
    // for its downloads to settle — then send a real mailbox event.
    let before = settled_downloads(&mut sync, &gateway);
    for _ in 0..3 {
        sync.step(Duration::from_millis(5)).unwrap();
    }
    assert_eq!(
        gateway.downloads.load(Ordering::SeqCst),
        before,
        "Quiet IDLE heartbeats must not download mail"
    );
    gateway.incoming.lock().unwrap().push(incoming(1));
    tx.send(Ok(true)).unwrap();
    until(&mut sync, || {
        context
            .read()
            .unwrap()
            .emails
            .iter()
            .any(|e| e.message_id == "new-1@example.com")
    });
}

#[test]
fn reconnect_refreshes_credentials_and_recovers_mail_missed_while_offline() {
    let gateway = Arc::new(Mail::default());
    let first = gateway.stream(1);
    let _second = gateway.stream(1);
    let context = setup(gateway.clone());
    let mut sync = coordinator(context.clone());
    until(&mut sync, || gateway.active.load(Ordering::SeqCst) == 1);
    for _ in 0..3 {
        sync.step(Duration::from_millis(5)).unwrap();
    }
    context
        .secrets
        .set(
            &account_key(&context.read().unwrap().accounts[0].email),
            "renewed-token",
        )
        .unwrap();
    gateway.incoming.lock().unwrap().push(incoming(1));
    drop(first);
    until(&mut sync, || {
        gateway
            .watched
            .lock()
            .unwrap()
            .contains(&(1, "renewed-token".into()))
            && context
                .read()
                .unwrap()
                .emails
                .iter()
                .any(|e| e.message_id == "new-1@example.com")
    });
    assert!(
        gateway
            .watched
            .lock()
            .unwrap()
            .contains(&(1, "renewed-token".into()))
    );
}

#[test]
fn added_accounts_receive_notifications_when_another_account_is_offline() {
    let gateway = Arc::new(Mail::default());
    let tx = gateway.stream(3);
    let context = setup(gateway.clone());
    let mut sync = coordinator(context.clone());
    sync.step(Duration::ZERO).unwrap();
    context
        .update(|store| {
            let mut account = store.accounts[0].clone();
            account.id = 3;
            account.email = "second@example.com".into();
            store.accounts.push(account);
            Ok(())
        })
        .unwrap();
    context
        .secrets
        .set(&account_key("second@example.com"), "second-token")
        .unwrap();
    until(&mut sync, || gateway.active.load(Ordering::SeqCst) == 1);
    gateway.incoming.lock().unwrap().push(incoming(3));
    tx.send(Ok(true)).unwrap();
    until(&mut sync, || {
        context
            .read()
            .unwrap()
            .emails
            .iter()
            .any(|e| e.message_id == "new-3@example.com")
    });
    assert!(
        !gateway
            .watched
            .lock()
            .unwrap()
            .iter()
            .any(|(id, _)| *id == 2),
        "Sample accounts must not open network watchers"
    );
    drop(sync);
    let deadline = Instant::now() + Duration::from_secs(1);
    while gateway.active.load(Ordering::SeqCst) != 0 {
        assert!(Instant::now() < deadline, "Watcher did not stop");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn periodic_sync_recovers_changes_even_without_an_idle_notification() {
    let gateway = Arc::new(Mail::default());
    let _tx = gateway.stream(1);
    let context = setup(gateway.clone());
    let mut sync = coordinator(context.clone());
    until(&mut sync, || gateway.active.load(Ordering::SeqCst) == 1);
    settled_downloads(&mut sync, &gateway);
    gateway.incoming.lock().unwrap().push(incoming(1));
    sync.next_fallback = Instant::now();
    assert!(
        sync.step(Duration::ZERO).unwrap().updated,
        "the fallback dispatches every syncable account"
    );
    until(&mut sync, || {
        context
            .read()
            .unwrap()
            .emails
            .iter()
            .any(|e| e.message_id == "new-1@example.com")
    });
}

#[test]
fn removed_accounts_stop_their_notification_connections() {
    let gateway = Arc::new(Mail::default());
    let _tx = gateway.stream(1);
    let context = setup(gateway.clone());
    let mut sync = coordinator(context.clone());
    until(&mut sync, || gateway.active.load(Ordering::SeqCst) == 1);
    context
        .update(|store| {
            store.accounts.retain(|a| a.id != 1);
            Ok(())
        })
        .unwrap();
    until(&mut sync, || gateway.active.load(Ordering::SeqCst) == 0);
}

#[test]
fn quiet_sessions_are_renewed_and_caught_up_before_their_lifetime_expires() {
    let gateway = Arc::new(Mail::default());
    let _first = gateway.stream(1);
    let _second = gateway.stream(1);
    let context = setup(gateway.clone());
    let mut sync = coordinator(context);
    sync.timing.session_lifetime = Duration::from_millis(20);
    until(&mut sync, || gateway.watched.lock().unwrap().len() >= 2);
    // The renewal connected (two watch attempts recorded). What must hold is
    // that the expired session is dropped, not the exact instant the counters
    // cross: the fake records the attempt before the connection counts as
    // active, and once the scripted streams run out the watcher retries
    // offline, so `active` settles at most one, never two.
    let deadline = Instant::now() + Duration::from_secs(3);
    while gateway.active.load(Ordering::SeqCst) > 1 {
        assert!(
            Instant::now() < deadline,
            "the expired session stayed active beside the renewed one"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Brief 3.3 step 1: two dirty accounts, the first account's gateway fails —
/// the second is still synced, because each download runs on its own worker
/// (the inline sequential sync returned the first error and never reached it).
#[test]
fn a_failing_account_does_not_stop_the_other_accounts_sync() {
    let gateway = Arc::new(Mail::default());
    let context = setup(gateway.clone());
    context
        .update(|store| {
            let mut account = store.accounts[0].clone();
            account.id = 3;
            account.email = "second@example.com".into();
            account.last_sync = None;
            store.accounts.push(account);
            Ok(())
        })
        .unwrap();
    context
        .secrets
        .set(&account_key("second@example.com"), "second-token")
        .unwrap();
    gateway.fail_sync.lock().unwrap().push(1);
    gateway.incoming.lock().unwrap().push(incoming(3));
    let mut sync = coordinator(context.clone());
    // Both accounts are dirty from spawn; account 1's worker fails and reports
    // on its own, while account 3's download completes in the same window.
    until(&mut sync, || {
        context
            .read()
            .unwrap()
            .emails
            .iter()
            .any(|e| e.message_id == "new-3@example.com")
    });
}
