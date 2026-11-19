//! Typed events the backend announces to the UI. They are emitted once, from the
//! writer, after the repository commit (ADR-0007 decision 5, INV-004): every
//! event carries the account and the ids it touched, never an empty payload.
use super::mail_sync_status::SyncPhase;
use crate::domain::undo::UndoAction;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AppEvent {
    EmailChanged { account_id: u32, ids: Vec<u64> },
    CountsChanged { account_id: u32 },
    UndoChanged { action: UndoAction },
    UndoRemoved { id: u64 },
    DraftsChanged { ids: Vec<u64> },
    SyncProgress { account_id: u32, phase: SyncPhase, added: usize },
    IndexProgress { account_id: u32, percent: u8 },
    /// The indexing of an account waits for the compute server (a transient pause: server busy
    /// or unreachable, model downloading, memory) or no longer: the state of its activity in
    /// the single indicator (INV-6), never an error notice.
    IndexWaiting { account_id: u32, waiting: bool },
    /// History backfill of an account (W3.1): messages brought into the list
    /// of those in All Mail; `complete` closes the activity.
    HistoryProgress { account_id: u32, fetched: u32, total: u32, complete: bool },
    /// The history brought `count` unread messages into the Inbox (EX-9): the
    /// interface asks once whether to mark the old ones read.
    HistoryUnread { account_id: u32, count: u32 },
    /// Search model install progress (ADR-0007 decision 7): `percent` is `None`
    /// while the size is unknown, and `done` closes the install — a success
    /// (100) and a failure (`None`) alike. Errors themselves stay separate
    /// notices, never indicator content (INV-009).
    ModelProgress { percent: Option<u8>, done: bool },
    /// Contacts synchronization: `account_id` is `None` for the whole run and
    /// the account of the book being read while it moves; `running` false ends it.
    ContactsSync { account_id: Option<u32>, running: bool },
    Notice { text: String },
    /// Mailboxes were added, changed or removed by the synchronization with the server.
    AccountsChanged,
    /// Rows of the activity log were added or corrected (centro attività): the
    /// ids are their `seq`. The UI re-reads the few rows it shows, so the same
    /// event serves this process and, through the change feed, the MCP's.
    ActivityChanged { ids: Vec<u64> },
}
impl AppEvent {
    /// Whether the event announces an empty change: `EmailChanged` or
    /// `DraftsChanged` without ids (INV-004: no event with an empty payload).
    /// `ServiceContext::update_with_events` drops these, keeping the order of
    /// the events that remain.
    pub fn is_empty_change(&self) -> bool {
        match self {
            Self::EmailChanged { ids, .. }
            | Self::DraftsChanged { ids }
            | Self::ActivityChanged { ids } => ids.is_empty(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        application::{
            AppError, AppResult, ServiceContext,
            ports::{EventSink, StoreRepository},
        },
        domain::{
            model::{Email, Store},
            undo::{UndoAction, UndoOperation, UndoStatus},
        },
        infrastructure::{
            gateways::{GmailGateway, KeyringSecrets, LocalAttachments, SystemClock},
            repository::MemoryRepository,
        },
    };
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    /// Repository that reports, at any moment, whether a transaction is open and
    /// how many commits have completed, so a sink can observe *when* it is called.
    struct Probe {
        inner: MemoryRepository,
        in_transaction: AtomicBool,
        commits: AtomicUsize,
        fail_commit: AtomicBool,
    }
    impl Probe {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                inner: MemoryRepository::new(Store::default()),
                in_transaction: AtomicBool::new(false),
                commits: AtomicUsize::new(0),
                fail_commit: AtomicBool::new(false),
            })
        }
    }
    impl StoreRepository for Probe {
        fn read(&self) -> AppResult<Store> {
            self.inner.read()
        }
        fn transaction(&self, op: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
            self.in_transaction.store(true, Ordering::SeqCst);
            let result = if self.fail_commit.load(Ordering::SeqCst) {
                // The operation runs, then the commit itself fails: nothing is saved.
                let mut scratch = self.inner.read()?;
                op(&mut scratch).and_then(|()| Err(AppError::storage("commit failed")))
            } else {
                self.inner.transaction(op)
            };
            self.in_transaction.store(false, Ordering::SeqCst);
            if result.is_ok() {
                self.commits.fetch_add(1, Ordering::SeqCst);
            }
            result
        }
    }

    /// One recorded emission: the event, whether a transaction was open, and the
    /// number of commits completed at that instant.
    type Seen = (AppEvent, bool, usize);
    struct Recorder {
        probe: Arc<Probe>,
        seen: Mutex<Vec<Seen>>,
    }
    impl EventSink for Recorder {
        fn emit(&self, event: AppEvent) {
            let open = self.probe.in_transaction.load(Ordering::SeqCst);
            let commits = self.probe.commits.load(Ordering::SeqCst);
            self.seen.lock().unwrap().push((event, open, commits));
        }
    }
    impl Recorder {
        fn events(&self) -> Vec<AppEvent> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|(event, _, _)| event.clone())
                .collect()
        }
    }

    fn context() -> (ServiceContext, Arc<Probe>, Arc<Recorder>) {
        let probe = Probe::new();
        let recorder = Arc::new(Recorder {
            probe: probe.clone(),
            seen: Mutex::new(Vec::new()),
        });
        let context = ServiceContext { compute: std::sync::Arc::new(crate::application::compute::NoCompute),
            repository: probe.clone(),
            mail: Arc::new(GmailGateway::new(
                std::env::temp_dir().join("qamille-events-test"),
            )),
            secrets: Arc::new(KeyringSecrets),
            clock: Arc::new(SystemClock),
            attachments: Arc::new(LocalAttachments),
            events: recorder.clone(),
        };
        (context, probe, recorder)
    }

    #[test]
    fn events_are_emitted_only_after_commit() {
        let (context, probe, recorder) = context();
        let value = context
            .update_with_events(|store| {
                // Nothing may have been announced before or inside the transaction.
                assert!(recorder.seen.lock().unwrap().is_empty());
                store.emails.push(Email {
                    id: 71,
                    account_id: 2,
                    ..Default::default()
                });
                Ok((
                    "done",
                    vec![
                        AppEvent::EmailChanged {
                            account_id: 2,
                            ids: vec![71],
                        },
                        AppEvent::CountsChanged { account_id: 2 },
                    ],
                ))
            })
            .unwrap();
        assert_eq!(value, "done");
        assert_eq!(probe.commits.load(Ordering::SeqCst), 1);
        assert_eq!(context.read().unwrap().emails.len(), 1);
        // Both events saw a closed transaction and exactly one completed commit.
        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        for (_, open, commits) in seen.iter() {
            assert!(!open, "an event was emitted inside the transaction");
            assert_eq!(*commits, 1, "an event was emitted before the commit");
        }
    }

    #[test]
    fn events_keep_the_order_the_operation_returned_them() {
        let (context, _, recorder) = context();
        let expected = vec![
            AppEvent::DraftsChanged { ids: vec![3] },
            AppEvent::UndoRemoved { id: 9 },
            AppEvent::EmailChanged {
                account_id: 1,
                ids: vec![5, 4],
            },
            AppEvent::Notice {
                text: "Moved".into(),
            },
        ];
        context
            .update_with_events(|_| Ok(((), expected.clone())))
            .unwrap();
        assert_eq!(recorder.events(), expected);
    }

    #[test]
    fn change_events_without_ids_are_dropped_keeping_the_order() {
        let (context, _, recorder) = context();
        context
            .update_with_events(|_| {
                Ok((
                    (),
                    vec![
                        AppEvent::DraftsChanged { ids: vec![] },
                        AppEvent::EmailChanged {
                            account_id: 1,
                            ids: vec![],
                        },
                        AppEvent::CountsChanged { account_id: 1 },
                        AppEvent::EmailChanged {
                            account_id: 1,
                            ids: vec![5],
                        },
                    ],
                ))
            })
            .unwrap();
        assert_eq!(
            recorder.events(),
            vec![
                AppEvent::CountsChanged { account_id: 1 },
                AppEvent::EmailChanged {
                    account_id: 1,
                    ids: vec![5]
                },
            ],
            "INV-004: no empty payload reaches the UI, the rest keeps its order"
        );
    }

    #[test]
    fn a_failed_operation_emits_nothing_and_saves_nothing() {
        let (context, probe, recorder) = context();
        let error = context
            .update_with_events::<()>(|store| {
                store.emails.push(Email {
                    id: 1,
                    ..Default::default()
                });
                Err(AppError::new("boom", "the operation failed"))
            })
            .unwrap_err();
        assert_eq!(error.code, "boom");
        assert!(recorder.events().is_empty());
        assert_eq!(probe.commits.load(Ordering::SeqCst), 0);
        assert!(context.read().unwrap().emails.is_empty());
    }

    #[test]
    fn a_failed_commit_emits_nothing() {
        let (context, probe, recorder) = context();
        probe.fail_commit.store(true, Ordering::SeqCst);
        let result = context
            .update_with_events(|_| Ok(((), vec![AppEvent::CountsChanged { account_id: 1 }])));
        assert!(result.is_err());
        assert!(recorder.events().is_empty());
    }

    #[test]
    fn email_changed_serializes_with_a_kebab_case_kind() {
        let event = AppEvent::EmailChanged {
            account_id: 2,
            ids: vec![71],
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"kind":"email-changed","account_id":2,"ids":[71]}"#
        );
    }

    #[test]
    fn every_variant_carries_its_kebab_case_kind() {
        use crate::application::mail_sync_status::SyncPhase;
        let undo = UndoAction {
            id: 4,
            operation: UndoOperation::DiscardDraft { draft_id: 8 },
            subject: "Draft".into(),
            created_at_ms: 1,
            expires_at_ms: 2,
            status: UndoStatus::Pending,
            error: None,
            retry_at_ms: None,
            attempts: 0,
        };
        let kinds = [
            (AppEvent::CountsChanged { account_id: 1 }, "counts-changed"),
            (AppEvent::UndoChanged { action: undo }, "undo-changed"),
            (AppEvent::UndoRemoved { id: 4 }, "undo-removed"),
            (AppEvent::DraftsChanged { ids: vec![8] }, "drafts-changed"),
            (
                AppEvent::SyncProgress {
                    account_id: 1,
                    phase: SyncPhase::Syncing,
                    added: 3,
                },
                "sync-progress",
            ),
            (
                AppEvent::IndexProgress {
                    account_id: 1,
                    percent: 40,
                },
                "index-progress",
            ),
            (
                AppEvent::IndexWaiting {
                    account_id: 1,
                    waiting: true,
                },
                "index-waiting",
            ),
            (
                AppEvent::ModelProgress {
                    percent: Some(42),
                    done: false,
                },
                "model-progress",
            ),
            (
                AppEvent::ContactsSync {
                    account_id: Some(2),
                    running: true,
                },
                "contacts-sync",
            ),
            (AppEvent::Notice { text: "x".into() }, "notice"),
            (AppEvent::ActivityChanged { ids: vec![1] }, "activity-changed"),
        ];
        for (event, kind) in kinds {
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["kind"], kind);
        }
    }

    /// The two Task 2.4 variants serialize exactly like the TypeScript mirror in
    /// desktop/src/lib/types.ts (`AppEvent`): `Option` fields are `null`, never
    /// missing keys, or `parseAppEvent` would reject them and force a reload.
    #[test]
    fn model_progress_and_contacts_sync_serialize_like_the_typescript_mirror() {
        assert_eq!(
            serde_json::to_string(&AppEvent::ModelProgress {
                percent: None,
                done: true
            })
            .unwrap(),
            r#"{"kind":"model-progress","percent":null,"done":true}"#
        );
        assert_eq!(
            serde_json::to_string(&AppEvent::ModelProgress {
                percent: Some(7),
                done: false
            })
            .unwrap(),
            r#"{"kind":"model-progress","percent":7,"done":false}"#
        );
        assert_eq!(
            serde_json::to_string(&AppEvent::ContactsSync {
                account_id: None,
                running: false
            })
            .unwrap(),
            r#"{"kind":"contacts-sync","account_id":null,"running":false}"#
        );
        assert_eq!(
            serde_json::to_string(&AppEvent::ContactsSync {
                account_id: Some(2),
                running: true
            })
            .unwrap(),
            r#"{"kind":"contacts-sync","account_id":2,"running":true}"#
        );
    }
}
