//! Scale test of the mail store (W2.1): 100,000 messages in SQLite.
//! Heavy: run alone, in release, and read the table it prints:
//!   cargo test --jobs 2 --release --no-default-features --test scale -- --ignored --nocapture
//! It fails when a measure exceeds the budget of the spec's goal 5 by more than 50%.
use qamille::{
    application::{
        AppResult, ServiceContext,
        mail::{MailQuery, MailService},
        history,
        ports::{
            AttachmentGateway, Clock, HistoryBatch, HistoryRequest, MailGateway, NullSink,
            OutgoingMessage, RemoteAction, SecretStore, StoreRepository, SyncBatch,
        },
        workspace::WorkspaceService,
    },
    domain::model::{Account, AccountKind, Attachment, Email, RemoteLocation, Store},
    infrastructure::sqlite_store::SqliteStoreRepository,
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// All Mail of `messages` UIDs for account 1, served by the history like Gmail would.
struct NoMail {
    messages: u32,
}
impl MailGateway for NoMail {
    fn history_batch(&self, _: &Account, request: &HistoryRequest) -> AppResult<HistoryBatch> {
        let mut progress = request.progress.clone();
        if progress.uid_validity != Some(7) {
            progress.restart(7);
        }
        let uid_next = self.messages + 1;
        let range = progress.plan(uid_next, request.batch_uids);
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let emails = range
            .map(|range| {
                (range.start..=range.end)
                    .map(|uid| {
                        let mut email = message(u64::from(uid) * 3, &mut rng);
                        email.account_id = 1;
                        email.uid = Some(uid);
                        email.remote_locations.clear();
                        email
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(HistoryBatch { folder: "[Gmail]/All Mail".into(), uid_validity: 7, uid_next, exists: self.messages, range, emails, ..Default::default() })
    }
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}
struct Secrets;
impl SecretStore for Secrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Ok("scale".into())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> i64 {
        1_800_000_000
    }
}
struct NoFiles;
impl AttachmentGateway for NoFiles {
    fn total_size(&self, paths: &[String]) -> AppResult<u64> {
        Ok(paths.len() as u64)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}

/// A deterministic pseudo-random sequence, so every run measures the same store.
struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % bound.max(1)
    }
}

fn message(id: u64, rng: &mut Rng) -> Email {
    let account_id = 1 + (id % 3) as u32;
    let folder = match rng.next(100) {
        0..60 => "Archive",
        60..85 => "Inbox",
        85..95 => "Sent",
        _ => "Trash",
    };
    let labels = (0..rng.next(4)).map(|n| format!("Label {}", (id + n) % 40)).collect();
    let attachments = (0..rng.next(4))
        .map(|n| Attachment {
            id: id * 1000 + n,
            name: format!("document-{n}.pdf"),
            mime: "application/pdf".into(),
            size: 10_000 + rng.next(2_000_000),
            path: None,
            inline_text: None,
        })
        .collect();
    let remote_locations = (0..1 + rng.next(2))
        .map(|n| RemoteLocation { folder: if n == 0 { "[Gmail]/All Mail".into() } else { "INBOX".into() }, uid: (id * 2 + n) as u32, uid_validity: Some(7) })
        .collect();
    Email {
        id,
        account_id,
        folder: folder.into(),
        thread_id: format!("thread-{}@example.com", id / (1 + rng.next(8))),
        message_id: format!("m{id}@example.com"),
        from_name: format!("Sender {}", id % 997),
        from_addr: format!("sender{}@example{}.com", id % 997, id % 13),
        to: vec![format!("me{account_id}@example.com")],
        subject: format!("Subject {} about invoice {}", id, id % 311),
        date: 1_800_000_000 - rng.next(10 * 365 * 86_400) as i64,
        read: rng.next(100) >= 20,
        labels,
        attachments,
        remote_locations,
        uid: Some(id as u32 * 2),
        remote_folder: Some("[Gmail]/All Mail".into()),
        remote_uid_validity: Some(7),
        remote_global_id: Some(1_000_000_000 + id),
        remote_only: true,
        ..Default::default()
    }
}

fn build(n: usize) -> (Scratch, Arc<SqliteStoreRepository>, Arc<ServiceContext>) {
    build_with(n, 0)
}
/// `n` messages written at once, plus a history of `history` messages served by the gateway.
fn build_with(n: usize, history: u32) -> (Scratch, Arc<SqliteStoreRepository>, Arc<ServiceContext>) {
    let root = std::env::temp_dir().join(format!("qamille-scale-{}-{n}-{history}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("data")).unwrap();
    let scratch = Scratch(root.clone());
    let repository = Arc::new(SqliteStoreRepository::open(root.join("data").join("store.sqlite")).unwrap());
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    repository
        .transaction(&mut |store: &mut Store| {
            for id in 1..=3 {
                store.accounts.push(Account {
                    id,
                    kind: AccountKind::Gmail,
                    provider_id: "gmail".into(),
                    email: format!("me{id}@example.com"),
                    username: format!("me{id}@example.com"),
                    name: format!("Account {id}"),
                    ..Default::default()
                });
            }
            store.emails = (1..=n as u64).map(|id| message(id, &mut rng)).collect();
            store.next_id = n as u64 + 1;
            store.settings.time_zone = "Europe/Rome".into();
            store.settings.locale = "it-IT".into();
            Ok(())
        })
        .unwrap();
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: repository.clone(),
        mail: Arc::new(NoMail { messages: history }),
        secrets: Arc::new(Secrets),
        clock: Arc::new(FixedClock),
        attachments: Arc::new(NoFiles),
    });
    (scratch, repository, context)
}

/// Median of five timed runs after one untimed run.
fn measure(mut work: impl FnMut()) -> Duration {
    work();
    let mut runs: Vec<Duration> = (0..5)
        .map(|_| {
            let started = Instant::now();
            work();
            started.elapsed()
        })
        .collect();
    runs.sort();
    runs[2]
}

fn resident_mb() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096 / 1024 / 1024)
}

/// Measures the operations of goal 5 on `context` and fails when one is 50% over its budget.
fn within_budget(repository: &SqliteStoreRepository, context: &Arc<ServiceContext>, before: Option<u64>) {
    let mail = MailService::new(context.clone());
    let workspace = WorkspaceService::new(context.clone());
    let mut next = context.read().unwrap().next_id;
    let target = context.read().unwrap().emails[500].id;
    let mut rng = Rng(42);
    let inbox = |threads: bool| MailQuery { folder: "Inbox".into(), limit: 60, threads, ..Default::default() };
    // (name, measured, budget from goal 5; None = report only)
    let rows: Vec<(&str, Duration, Option<Duration>)> = vec![
        ("read()", measure(|| drop(repository.read().unwrap())), None),
        ("read_shared()", measure(|| drop(context.read_shared().unwrap())), None),
        ("list Inbox, 60 rows", measure(|| drop(mail.list(inbox(false)).unwrap())), Some(Duration::from_millis(150))),
        ("list Inbox, conversations", measure(|| drop(mail.list(inbox(true)).unwrap())), Some(Duration::from_millis(150))),
        ("workspace_snapshot", measure(|| drop(workspace.snapshot().unwrap())), Some(Duration::from_millis(100))),
        (
            "mark one message read",
            measure(|| {
                // The API of the mail actions (`MailService::commit_actions`):
                // the write names the one message it changes.
                context
                    .update_emails(&[target], |store| {
                        let email = store.emails.iter_mut().find(|e| e.id == target).unwrap();
                        email.read = !email.read;
                        Ok(())
                    })
                    .unwrap()
            }),
            Some(Duration::from_millis(50)),
        ),
        (
            "insert 25 metadata rows",
            measure(|| {
                let rows: Vec<Email> = (0..25).map(|_| { next += 1; message(next, &mut rng) }).collect();
                context.update(|store| { store.next_id = next + 1; store.emails.extend(rows.iter().cloned()); Ok(()) }).unwrap()
            }),
            Some(Duration::from_millis(250)),
        ),
        (
            "insert 500 metadata rows",
            measure(|| {
                let rows: Vec<Email> = (0..500).map(|_| { next += 1; message(next, &mut rng) }).collect();
                context.update(|store| { store.next_id = next + 1; store.emails.extend(rows.iter().cloned()); Ok(()) }).unwrap()
            }),
            Some(Duration::from_millis(250)),
        ),
        (
            "search one word",
            measure(|| drop(mail.list(MailQuery { search: "invoice".into(), limit: 60, ..Default::default() }).unwrap())),
            None,
        ),
    ];
    let after = resident_mb();
    println!("\n| measure | median | budget |\n| --- | ---: | ---: |");
    for (name, took, budget) in &rows {
        println!("| {name} | {} ms | {} |", took.as_millis(), budget.map_or("—".into(), |b| format!("{} ms", b.as_millis())));
    }
    if let (Some(before), Some(after)) = (before, after) {
        println!("| resident memory | {before} MB → {after} MB (+{}) | +300 MB |", after.saturating_sub(before));
    }
    let over: Vec<String> = rows
        .iter()
        .filter_map(|(name, took, budget)| {
            let budget = (*budget)?;
            (took.as_secs_f64() > budget.as_secs_f64() * 1.5).then(|| format!("{name}: {} ms", took.as_millis()))
        })
        .collect();
    assert!(over.is_empty(), "over budget (+50%): {over:?}");
}

#[test]
#[ignore = "heavy: run alone with --release --ignored --nocapture"]
fn one_hundred_thousand_messages_stay_within_budget() {
    let before = resident_mb();
    let started = Instant::now();
    let (_scratch, repository, context) = build(100_000);
    println!("built 100,000 messages in {:?}", started.elapsed());
    within_budget(&repository, &context, before);
}

#[test]
#[ignore = "heavy: run alone with --release --ignored --nocapture"]
fn one_hundred_thousand_messages_from_the_history_stay_within_budget() {
    // Gate of wave 3: the same budgets once the history has brought 100,000 rows in batches.
    let before = resident_mb();
    let (_scratch, repository, context) = build_with(0, 100_000);
    let started = Instant::now();
    let mut batches = 1;
    while history::run_batch(&context, 1).unwrap() {
        batches += 1;
    }
    let store = context.read().unwrap();
    println!("history of 100,000 messages in {batches} batches: {:?}", started.elapsed());
    assert_eq!(store.emails.len(), 100_000);
    assert!(store.accounts[0].history.complete);
    drop(store);
    within_budget(&repository, &context, before);
}
